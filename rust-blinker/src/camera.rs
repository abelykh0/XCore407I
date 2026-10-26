//! Static 512x512 NV12 test image and UVC payload packetizer.
//!
//! The image is generated on demand, so it does not consume a 384 KiB frame
//! buffer. The payload layout matches the original project's NV12 source.

pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 512;
pub const Y_PLANE_BYTES: usize = WIDTH * HEIGHT;
pub const FRAME_BYTES: usize = Y_PLANE_BYTES + (WIDTH * HEIGHT / 2);
pub const UVC_HEADER_BYTES: usize = 4;
pub const PAYLOAD_BYTES: usize = WIDTH * 3 / 2;
pub const TRANSFER_BYTES: usize = UVC_HEADER_BYTES + PAYLOAD_BYTES;

// SMPTE-style vertical bars: white, yellow, cyan, green, magenta, red, blue,
// black, represented as limited-range BT.601 Y, Cb, Cr values.
const Y: [u8; 8] = [235, 210, 170, 145, 106, 81, 41, 16];
const U: [u8; 8] = [128, 16, 166, 54, 202, 90, 240, 128];
const V: [u8; 8] = [128, 146, 16, 34, 222, 240, 110, 128];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Packet {
    pub len: usize,
    pub frame_complete: bool,
}

/// Emits the same static color-bar frame repeatedly as UVC payloads.
pub struct Nv12ColorBars {
    offset: usize,
    frame_id: bool,
}

impl Nv12ColorBars {
    pub const fn new() -> Self {
        Self {
            offset: 0,
            frame_id: false,
        }
    }

    /// Fill one UVC payload into caller-owned storage.
    ///
    /// `out` must hold at least `TRANSFER_BYTES` bytes. Bytes 0..4 are the
    /// UVC payload header; the remaining bytes are a chunk of the NV12 frame.
    pub fn next_packet(&mut self, out: &mut [u8]) -> Option<Packet> {
        if out.len() < UVC_HEADER_BYTES {
            return None;
        }

        let remaining = FRAME_BYTES - self.offset;
        let data_len = remaining
            .min(PAYLOAD_BYTES)
            .min(out.len() - UVC_HEADER_BYTES);
        if data_len == 0 {
            return None;
        }

        let complete = data_len == remaining;
        out[0] = UVC_HEADER_BYTES as u8;
        // EOH terminates this four-byte header; FID toggles per frame and
        // EOF marks the last payload of each frame.
        out[1] = 0x80 | u8::from(self.frame_id) | if complete { 0x02 } else { 0 };
        out[2] = 0; // PTS not present
        out[3] = 0; // SCR not present
        fill_nv12(
            self.offset,
            &mut out[UVC_HEADER_BYTES..UVC_HEADER_BYTES + data_len],
        );

        self.offset += data_len;
        if complete {
            self.offset = 0;
            self.frame_id = !self.frame_id;
        }

        Some(Packet {
            len: UVC_HEADER_BYTES + data_len,
            frame_complete: complete,
        })
    }
}

impl Default for Nv12ColorBars {
    fn default() -> Self {
        Self::new()
    }
}

/// Copy a byte range from the static NV12 color-bar image.
pub fn fill_nv12(offset: usize, out: &mut [u8]) {
    for (i, byte) in out.iter_mut().enumerate() {
        let pos = offset + i;
        if pos >= FRAME_BYTES {
            *byte = 0;
        } else if pos < Y_PLANE_BYTES {
            let x = pos % WIDTH;
            *byte = Y[x / (WIDTH / 8)];
        } else {
            let chroma_pos = pos - Y_PLANE_BYTES;
            let chroma_x = (chroma_pos / 2) % (WIDTH / 2);
            let bar = (chroma_x * 2) / (WIDTH / 8);
            *byte = if chroma_pos & 1 == 0 { U[bar] } else { V[bar] };
        }
    }
}
