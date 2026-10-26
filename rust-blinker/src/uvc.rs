//! UVC 1.1 uncompressed NV12 video function.
//!
//! This is the class layer shared with the USB device controller: it builds
//! the UVC configuration descriptors, answers Probe/Commit control requests,
//! and supplies one 772-byte isochronous payload per IN completion. The USB
//! controller glue must expose interface 0/1 and an HS ISO IN endpoint at
//! `ENDPOINT_IN` with `MAX_PACKET_SIZE` bytes per microframe.

use crate::camera::{Nv12ColorBars, TRANSFER_BYTES};

pub const CONTROL_INTERFACE: u8 = 0;
pub const STREAMING_INTERFACE: u8 = 1;
pub const ENDPOINT_IN: u8 = 0x81;
pub const MAX_PACKET_SIZE: u16 = TRANSFER_BYTES as u16;
pub const FRAME_INTERVAL_100NS: u32 = 10_000_000 / 12;
pub const FRAME_BYTES: u32 = 512 * 512 * 3 / 2;
pub const UVC_VERSION: u16 = 0x0110;
pub const PROBE_LENGTH: usize = 34;
pub const PROBE_LENGTH_UVC_10: usize = 26;

const PROBE: u8 = 1;
const COMMIT: u8 = 2;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;
const GET_LEN: u8 = 0x85;
const GET_INFO: u8 = 0x86;
const GET_DEF: u8 = 0x87;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    Unsupported,
    InvalidLength,
    InvalidInterface,
    InvalidSelector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlResponse<'a> {
    Data(&'a [u8]),
    Ack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamState {
    Stopped,
    Ready,
    Streaming,
}

/// UVC function state. Call `on_sof`, then `next_in_packet` after each
/// successful isochronous IN completion, matching the original STM32 class.
pub struct UvcClass {
    state: StreamState,
    alternate_setting: u8,
    probe: [u8; PROBE_LENGTH],
    commit: [u8; PROBE_LENGTH],
    camera: Nv12ColorBars,
    packet: [u8; TRANSFER_BYTES],
    start_marker_pending: bool,
}

impl UvcClass {
    pub const fn new() -> Self {
        Self {
            state: StreamState::Stopped,
            alternate_setting: 0,
            probe: make_stream_control(),
            commit: make_stream_control(),
            camera: Nv12ColorBars::new(),
            packet: [0; TRANSFER_BYTES],
            start_marker_pending: false,
        }
    }

    pub const fn alternate_setting(&self) -> u8 {
        self.alternate_setting
    }

    pub const fn is_streaming(&self) -> bool {
        matches!(self.state, StreamState::Streaming)
    }

    /// Handle standard SET_INTERFACE for the VideoStreaming interface.
    pub fn set_interface(&mut self, interface: u8, alternate: u8) -> Result<(), ControlError> {
        if interface != STREAMING_INTERFACE {
            return Err(ControlError::InvalidInterface);
        }
        match alternate {
            0 => {
                self.alternate_setting = 0;
                self.state = StreamState::Stopped;
                self.start_marker_pending = false;
                self.camera = Nv12ColorBars::new();
                Ok(())
            }
            1 => {
                self.alternate_setting = 1;
                self.state = StreamState::Ready;
                self.start_marker_pending = true;
                self.camera = Nv12ColorBars::new();
                Ok(())
            }
            _ => Err(ControlError::Unsupported),
        }
    }

    /// Produces the first complete UVC payload when alternate setting 1
    /// activates the stream. The USB driver queues it immediately.
    pub fn start_stream_packet(&mut self) -> Option<&[u8]> {
        if self.state == StreamState::Ready && self.start_marker_pending {
            self.start_marker_pending = false;
            self.state = StreamState::Streaming;
            let packet = self.camera.next_packet(&mut self.packet)?;
            Some(&self.packet[..packet.len])
        } else {
            None
        }
    }

    /// Return the next UVC payload after the previous IN transfer completes.
    /// The returned slice remains valid until this method is called again.
    pub fn next_in_packet(&mut self) -> Option<&[u8]> {
        if self.state != StreamState::Streaming {
            return None;
        }
        let packet = self.camera.next_packet(&mut self.packet)?;
        Some(&self.packet[..packet.len])
    }

    /// Process UVC class-specific control requests. `out_data` is the EP0 OUT
    /// data stage for SET_CUR. The endpoint-zero driver sends `Data` and
    /// completes the status stage for `Ack`.
    pub fn control<'a>(
        &'a mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        requested_len: u16,
        out_data: &[u8],
    ) -> Result<ControlResponse<'a>, ControlError> {
        if request_type & 0x60 != 0x20 {
            return Err(ControlError::Unsupported);
        }
        let interface = index as u8;
        if interface != CONTROL_INTERFACE && interface != STREAMING_INTERFACE {
            return Err(ControlError::InvalidInterface);
        }

        // VideoControl Request Error Code Control (entity 2, selector 2).
        if interface == CONTROL_INTERFACE {
            if (value >> 8) == 2 && matches!(request, GET_CUR | GET_DEF) {
                return Ok(ControlResponse::Data(&[0x06]));
            }
            return Err(ControlError::Unsupported);
        }

        // UVC places the Control Selector in the high byte of wValue.
        let selector = (value >> 8) as u8;
        if selector != PROBE && selector != COMMIT {
            return Err(ControlError::InvalidSelector);
        }
        match request {
            GET_CUR | GET_DEF | GET_MIN | GET_MAX => {
                let len = (requested_len as usize).min(PROBE_LENGTH);
                if selector == PROBE {
                    Ok(ControlResponse::Data(&self.probe[..len]))
                } else {
                    Ok(ControlResponse::Data(&self.commit[..len]))
                }
            }
            GET_RES => Ok(ControlResponse::Data(
                &[0; PROBE_LENGTH][..(requested_len as usize).min(PROBE_LENGTH)],
            )),
            GET_LEN => Ok(ControlResponse::Data(
                &[PROBE_LENGTH as u8, 0][..(requested_len as usize).min(2)],
            )),
            GET_INFO => Ok(ControlResponse::Data(
                &[0x03][..(requested_len as usize).min(1)],
            )),
            0x01 => {
                if out_data.len() < PROBE_LENGTH_UVC_10 {
                    return Err(ControlError::InvalidLength);
                }
                let mut value = if selector == PROBE {
                    self.probe
                } else {
                    self.commit
                };
                let len = out_data.len().min(PROBE_LENGTH);
                value[..len].copy_from_slice(&out_data[..len]);
                // Fixed single format/frame: canonicalize host values to the
                // only mode supported, as in the original C implementation.
                normalize_stream_control(&mut value);
                if selector == COMMIT {
                    self.commit = value;
                } else {
                    self.probe = value;
                }
                Ok(ControlResponse::Ack)
            }
            _ => Err(ControlError::Unsupported),
        }
    }
}

impl Default for UvcClass {
    fn default() -> Self {
        Self::new()
    }
}

const fn make_stream_control() -> [u8; PROBE_LENGTH] {
    let mut d = [0u8; PROBE_LENGTH];
    d[2] = 1; // bFormatIndex
    d[3] = 1; // bFrameIndex
    d[4] = FRAME_INTERVAL_100NS as u8;
    d[5] = (FRAME_INTERVAL_100NS >> 8) as u8;
    d[6] = (FRAME_INTERVAL_100NS >> 16) as u8;
    d[7] = (FRAME_INTERVAL_100NS >> 24) as u8;
    d[18] = FRAME_BYTES as u8;
    d[19] = (FRAME_BYTES >> 8) as u8;
    d[20] = (FRAME_BYTES >> 16) as u8;
    d[21] = (FRAME_BYTES >> 24) as u8;
    d[22] = MAX_PACKET_SIZE as u8;
    d[23] = (MAX_PACKET_SIZE >> 8) as u8;
    let clock_hz = 48_000_000u32;
    d[26] = clock_hz as u8;
    d[27] = (clock_hz >> 8) as u8;
    d[28] = (clock_hz >> 16) as u8;
    d[29] = (clock_hz >> 24) as u8;
    d
}

fn normalize_stream_control(d: &mut [u8; PROBE_LENGTH]) {
    let canonical = make_stream_control();
    d[2..4].copy_from_slice(&canonical[2..4]);
    d[4..8].copy_from_slice(&canonical[4..8]);
    d[18..26].copy_from_slice(&canonical[18..26]);
    d[26..34].copy_from_slice(&canonical[26..34]);
}

/// Build the 158-byte configuration descriptor used by the original
/// two-interface UVC function. Returns None if the caller's buffer is small.
pub fn configuration_descriptor(out: &mut [u8]) -> Option<usize> {
    let mut w = DescriptorWriter { out, pos: 0 };
    // Configuration and Video Interface Association.
    w.desc(&[9, 2, 158, 0, 2, 1, 0, 0x80, 50])?;
    w.desc(&[8, 0x0b, 0, 2, 0x0e, 3, 0, 0])?;
    // VideoControl interface and UVC 1.1 header, input and output terminals.
    w.desc(&[9, 4, 0, 0, 0, 0x0e, 1, 0, 0])?;
    w.desc(&[13, 0x24, 1, 0x10, 0x01, 30, 0, 0x00, 0x6c, 0xdc, 0x02, 1, 1])?;
    w.desc(&[8, 0x24, 2, 1, 0x00, 0x02, 0, 0])?;
    w.desc(&[9, 0x24, 3, 2, 0x01, 0x01, 0, 1, 0])?;
    // VideoStreaming alternate 0 and class-specific format/frame descriptors.
    w.desc(&[9, 4, 1, 0, 0, 0x0e, 2, 0, 0])?;
    w.desc(&[14, 0x24, 1, 1, 77, 0, ENDPOINT_IN, 0, 2, 0, 0, 0, 1, 0])?;
    // Uncompressed NV12 (GUID {3231564E-0000-0010-8000-00AA00389B71}).
    w.desc(&[
        27, 0x24, 4, 1, 1, 0x4e, 0x56, 0x31, 0x32, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b,
        0x71, 12, 1, 0, 0, 0, 0,
    ])?;
    let mut frame = [0u8; 30];
    frame[0..5].copy_from_slice(&[30, 0x24, 5, 1, 0]);
    frame[5..7].copy_from_slice(&512u16.to_le_bytes());
    frame[7..9].copy_from_slice(&512u16.to_le_bytes());
    // Keep the original firmware's 16-bit-per-pixel rate estimate.
    let bitrate: u32 = 512 * 512 * 16 * 12;
    frame[9..13].copy_from_slice(&bitrate.to_le_bytes());
    frame[13..17].copy_from_slice(&bitrate.to_le_bytes());
    frame[17..21].copy_from_slice(&FRAME_BYTES.to_le_bytes());
    frame[21..25].copy_from_slice(&FRAME_INTERVAL_100NS.to_le_bytes());
    frame[25] = 1;
    frame[26..30].copy_from_slice(&FRAME_INTERVAL_100NS.to_le_bytes());
    w.desc(&frame)?;
    w.desc(&[6, 0x24, 0x0d, 1, 1, 4])?;
    // Alternate 1 is the high-speed isochronous IN stream endpoint.
    w.desc(&[9, 4, 1, 1, 1, 0x0e, 2, 0, 0])?;
    w.desc(&[
        7,
        5,
        ENDPOINT_IN,
        5,
        MAX_PACKET_SIZE as u8,
        (MAX_PACKET_SIZE >> 8) as u8,
        1,
    ])?;
    if w.pos != 158 {
        return None;
    }
    Some(w.pos)
}

struct DescriptorWriter<'a> {
    out: &'a mut [u8],
    pos: usize,
}
impl DescriptorWriter<'_> {
    fn desc(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.pos.checked_add(bytes.len())?;
        if end > self.out.len() {
            return None;
        }
        self.out[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        Some(())
    }
}
