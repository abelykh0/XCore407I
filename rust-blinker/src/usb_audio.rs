//! USB Audio Class 1.0 capture function for BK-0010 sound output.
//!
//! The BK emulator writes the speaker level to bit 6 of port 0177714.
//! Transitions are timestamped by the STM32 DWT counter and converted to
//! 48 kHz mono, 16-bit PCM for the USB isochronous IN endpoint.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use usb_device::bus::{InterfaceNumber, StringIndex, UsbBus, UsbBusAllocator};
use usb_device::class::UsbClass;
use usb_device::descriptor::DescriptorWriter;
use usb_device::endpoint::{
    EndpointAddress, EndpointIn, EndpointType, IsochronousSynchronizationType,
    IsochronousUsageType,
};
use usb_device::Result;

const AUDIO_CLASS: u8 = 0x01;
const AUDIO_CONTROL_SUBCLASS: u8 = 0x01;
const AUDIO_STREAMING_SUBCLASS: u8 = 0x02;
const AUDIO_ENDPOINT_IN: u8 = 0x82;
const AUDIO_PACKET_BYTES: u16 = 12;
const AUDIO_SAMPLES_PER_PACKET: usize = 6;
const AUDIO_SAMPLE_PERIOD_CYCLES: u32 = 3_500; // 168 MHz / 48 kHz
const AUDIO_STARTUP_LATENCY_CYCLES: u32 = 168_000; // 1 ms transition-queue lead

static AUDIO_ALT1_ACTIVE: AtomicBool = AtomicBool::new(false);
static AUDIO_ATTEMPT_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_SUBMIT_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_WRITE_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_COMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_INCOMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_INCOMPLETE_ADDRESS: AtomicU32 = AtomicU32::new(0);

/// Capture the DWC2 snapshots for both active HS isochronous IN endpoints.
/// The global IISOIXFR interrupt can involve either endpoint, so inspect EP1 and EP2.
pub fn endpoint_incomplete_snapshots() -> ((u32, u32, u32, u32), (u32, u32, u32, u32)) {
    (
        synopsys_usb_otg::iso_incomplete_snapshot(1),
        synopsys_usb_otg::iso_incomplete_snapshot(2),
    )
}

/// Raw endpoint address passed to the most recent incomplete callback.
pub fn incomplete_callback_address() -> u32 {
    AUDIO_INCOMPLETE_ADDRESS.load(Ordering::Relaxed)
}

/// Snapshot of the USB Audio stream state and activity counters.
pub fn diagnostics() -> (bool, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) {
    let edges = unsafe { bk_audio_edge_count() };
    let port_writes = unsafe { bk_audio_port_write_count() };
    let (diepctl, dieptsiz, diepint, fnsof) =
        synopsys_usb_otg::iso_incomplete_snapshot(2);
    (
        AUDIO_ALT1_ACTIVE.load(Ordering::Acquire),
        AUDIO_ATTEMPT_COUNT.load(Ordering::Relaxed),
        AUDIO_SUBMIT_COUNT.load(Ordering::Relaxed),
        AUDIO_WRITE_FAIL_COUNT.load(Ordering::Relaxed),
        AUDIO_COMPLETE_COUNT.load(Ordering::Relaxed),
        AUDIO_INCOMPLETE_COUNT.load(Ordering::Relaxed),
        edges,
        port_writes,
        diepctl,
        dieptsiz,
        diepint,
        fnsof,
    )
}

unsafe extern "C" {
    fn bk_audio_start() -> u8;
    fn bk_audio_stop();
    fn bk_audio_pop_edge(cycles: *mut u32, level: *mut u8) -> u8;
    fn bk_audio_edge_count() -> u32;
    fn bk_audio_port_write_count() -> u32;
}

/// USB Audio Class 1.0 capture function (device-to-host audio source).
pub struct AudioUsbClass<'a, B: UsbBus> {
    control_interface: InterfaceNumber,
    streaming_interface: InterfaceNumber,
    function_string: StringIndex,
    endpoint: EndpointIn<'a, B>,
    streaming_alt: u8,
    next_sample_cycle: u32,
    sample_level: bool,
    pending_edge: Option<(u32, bool)>,
    packet_queued: bool,
    packet_buffer: [u8; AUDIO_PACKET_BYTES as usize],
    incomplete_count: u8,
    halted_after_incomplete: bool,
}

impl<'a, B: UsbBus> AudioUsbClass<'a, B> {
    pub fn new(allocator: &'a UsbBusAllocator<B>) -> Self {
        let control_interface = allocator.interface();
        let streaming_interface = allocator.interface();
        let function_string = allocator.string();
        let endpoint = allocator
            .alloc(
                Some(EndpointAddress::from(AUDIO_ENDPOINT_IN)),
                EndpointType::Isochronous {
                    synchronization: IsochronousSynchronizationType::Asynchronous,
                    usage: IsochronousUsageType::Data,
                },
                AUDIO_PACKET_BYTES,
                1,
            )
            .expect("failed to allocate USB Audio isochronous IN endpoint");

        Self {
            control_interface,
            streaming_interface,
            function_string,
            endpoint,
            streaming_alt: 0,
            next_sample_cycle: 0,
            sample_level: false,
            pending_edge: None,
            packet_queued: false,
            packet_buffer: [0; AUDIO_PACKET_BYTES as usize],
            incomplete_count: 0,
            halted_after_incomplete: false,
        }
    }

    #[inline(always)]
    fn cycle_counter() -> u32 {
        cortex_m::peripheral::DWT::cycle_count()
    }

    fn apply_edges_through(&mut self, sample_cycle: u32) {
        loop {
            if self.pending_edge.is_none() {
                let mut cycles = 0u32;
                let mut level = 0u8;
                let available = unsafe { bk_audio_pop_edge(&mut cycles, &mut level) };
                if available == 0 {
                    break;
                }
                self.pending_edge = Some((cycles, level != 0));
            }

            let (edge_cycle, edge_level) = self.pending_edge.unwrap();
            // Signed wrapping subtraction is valid because the queue latency
            // and maximum backlog are far below half the 32-bit DWT period.
            if sample_cycle.wrapping_sub(edge_cycle) as i32 >= 0 {
                self.sample_level = edge_level;
                self.pending_edge = None;
            } else {
                break;
            }
        }
    }

    fn make_packet(&mut self) {
        for sample_index in 0..AUDIO_SAMPLES_PER_PACKET {
            let sample_cycle = self.next_sample_cycle;
            self.apply_edges_through(sample_cycle);

            // A simple square wave from the BK speaker output. Amplitude is
            // deliberately conservative; later we can tune level/filtering.
            let sample: i16 = if self.sample_level { 12_000 } else { -12_000 };
            let offset = sample_index * 2;
            self.packet_buffer[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());

            self.next_sample_cycle =
                self.next_sample_cycle.wrapping_add(AUDIO_SAMPLE_PERIOD_CYCLES);
        }

    }

    fn queue_current_packet(&mut self) {
        if self.streaming_alt != 1 || self.packet_queued {
            return;
        }
        AUDIO_ATTEMPT_COUNT.fetch_add(1, Ordering::Relaxed);
        // The 12-byte audio payload is copied directly into EP2's TX FIFO.
        // No DMA buffer or DMA lifetime constraint is involved.
        let result = self.endpoint.write_iso_batch_fifo(&self.packet_buffer, 1, 1);
        match result {
            Ok(_) => {
                self.packet_queued = true;
                AUDIO_SUBMIT_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.packet_queued = false;
                AUDIO_WRITE_FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn submit_packet(&mut self) {
        if self.streaming_alt != 1 || self.packet_queued || self.halted_after_incomplete {
            return;
        }
        self.make_packet();
        self.queue_current_packet();
    }

    fn write_audio_control_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(
            self.control_interface,
            AUDIO_CLASS,
            AUDIO_CONTROL_SUBCLASS,
            0,
        )?;

        // UAC1 AudioControl header. The class-specific AC descriptor collection
        // is 30 bytes: header (9), input terminal (12), and output terminal (9).
        writer.write(
            0x24,
            &[
                1, 0x00, 0x01, 0x1E, 0x00, 1,
                u8::from(self.streaming_interface),
            ],
        )?;

        // Input Terminal: host capture source, mono.
        writer.write(
            0x24,
            &[2, 1, 0x03, 0x06, 0, 1, 0x01, 0x00, 0, 0],
        )?;

        // Output Terminal: USB Streaming, fed by the BK audio source.
        writer.write(0x24, &[3, 3, 0x01, 0x01, 0, 1, 0])?;
        Ok(())
    }

    fn write_audio_streaming_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface_alt(
            self.streaming_interface,
            0,
            AUDIO_CLASS,
            AUDIO_STREAMING_SUBCLASS,
            0,
            Some(self.function_string),
        )?;

        writer.interface_alt(
            self.streaming_interface,
            1,
            AUDIO_CLASS,
            AUDIO_STREAMING_SUBCLASS,
            0,
            Some(self.function_string),
        )?;

        // AS_GENERAL: terminal link 3, no delay, PCM format tag 0x0001.
        writer.write(0x24, &[1, 3, 0, 1, 0])?;

        // FORMAT_TYPE_I: mono, 16-bit samples, one discrete rate (48 kHz).
        writer.write(0x24, &[2, 1, 1, 2, 16, 1, 0x80, 0xBB, 0x00])?;

        // HS isochronous IN: six mono 16-bit samples per 125-us microframe.
        writer.endpoint_ex(&self.endpoint, |buf| {
            if buf.len() < 2 {
                return Err(usb_device::UsbError::BufferOverflow);
            }
            buf[0] = 0; // bRefresh
            buf[1] = 0; // bSynchAddress
            Ok(2)
        })?;

        // Class-specific endpoint descriptor; no endpoint controls advertised.
        writer.write(0x25, &[1, 0, 0, 0, 0])?;
        Ok(())
    }
}

impl<B: UsbBus> UsbClass<B> for AudioUsbClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        // Keep the existing composite allocation: UVC interfaces 0-1 / EP 0x81,
        // Audio interfaces 2-3 / EP 0x82.
        writer.iad(
            self.control_interface,
            2,
            AUDIO_CLASS,
            0,
            0,
            Some(self.function_string),
        )?;
        self.write_audio_control_descriptors(writer)?;
        self.write_audio_streaming_descriptors(writer)
    }

    fn get_string(
        &self,
        index: StringIndex,
        _lang_id: usb_device::descriptor::lang_id::LangID,
    ) -> Option<&str> {
        if index == self.function_string {
            Some("XCore407I Audio")
        } else {
            None
        }
    }

    fn reset(&mut self) {
        unsafe { bk_audio_stop(); }
        AUDIO_ALT1_ACTIVE.store(false, Ordering::Release);
        self.streaming_alt = 0;
        self.pending_edge = None;
        self.packet_queued = false;
    }

    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        if interface == self.control_interface {
            Some(0)
        } else if interface == self.streaming_interface {
            Some(self.streaming_alt)
        } else {
            None
        }
    }

    fn set_alt_setting(&mut self, interface: InterfaceNumber, alternative: u8) -> bool {
        if interface == self.control_interface {
            return alternative == 0;
        }
        if interface != self.streaming_interface || alternative > 1 {
            return false;
        }

        self.streaming_alt = alternative;
        AUDIO_ALT1_ACTIVE.store(alternative == 1, Ordering::Release);
        if alternative == 1 {
            AUDIO_ATTEMPT_COUNT.store(0, Ordering::Relaxed);
            AUDIO_SUBMIT_COUNT.store(0, Ordering::Relaxed);
            AUDIO_WRITE_FAIL_COUNT.store(0, Ordering::Relaxed);
            AUDIO_COMPLETE_COUNT.store(0, Ordering::Relaxed);
            AUDIO_INCOMPLETE_COUNT.store(0, Ordering::Relaxed);
        }
        self.pending_edge = None;
        self.packet_queued = false;
        self.incomplete_count = 0;
        self.halted_after_incomplete = false;

        if alternative == 1 {
            self.sample_level = unsafe { bk_audio_start() != 0 };
            self.next_sample_cycle = Self::cycle_counter()
                .wrapping_sub(AUDIO_STARTUP_LATENCY_CYCLES);
            // Prime the endpoint immediately; subsequent packets are queued
            // from endpoint_in_complete once per HS microframe.
            self.submit_packet();
        } else {
            unsafe { bk_audio_stop(); }
        }

        true
    }

    fn poll(&mut self) {
        // Retry if the controller was not ready when the previous packet
        // was first submitted. Never queue a second packet while one is active.
        self.submit_packet();
    }

    fn endpoint_in_complete(&mut self, address: EndpointAddress) {
        if address == self.endpoint.address() {
            AUDIO_COMPLETE_COUNT.fetch_add(1, Ordering::Relaxed);
            self.incomplete_count = 0;
            self.packet_queued = false;
            self.submit_packet();
        }
    }

    fn endpoint_in_incomplete(&mut self, address: EndpointAddress) {
        AUDIO_INCOMPLETE_ADDRESS.store(u8::from(address) as u32, Ordering::Relaxed);
        if address != self.endpoint.address() {
            return;
        }

        AUDIO_INCOMPLETE_COUNT.fetch_add(1, Ordering::Relaxed);
        self.packet_queued = false;
        self.incomplete_count = self.incomplete_count.saturating_add(1);

        // Diagnostic isolation: stop the audio retry/recovery loop after the
        // first missed isochronous frame. A successful completion can still
        // keep the stream running; an incomplete callback halts submissions.
        self.halted_after_incomplete = true;
    }
}
