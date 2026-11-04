//! BK audio capture backend for the UAC2 Line In function.
//!
//! The BK emulator writes a one-bit speaker level to port 0177714 bit 6.
//! Transitions are timestamped with DWT CYCCNT and converted to mono,
//! signed 16-bit PCM at 48 kHz. The UAC2 descriptors expose this source as
//! an external Line Connector feeding a USB Streaming output terminal.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use usb_device::bus::UsbBus;
use usb_device::endpoint::{Endpoint, In, Out};
use usb_device::UsbDirection;
use usbd_uac2::descriptors::ClockType;
use usbd_uac2::{
    AudioHandler, ClockSource, RangeEntry, UsbAudioClassError, UsbIsochronousFeedback,
};

const AUDIO_SAMPLE_RATE: u32 = 48_000;
const AUDIO_SAMPLES_PER_MICROFRAME: usize = 6;
const AUDIO_PACKET_BYTES: usize = AUDIO_SAMPLES_PER_MICROFRAME * 2;
const AUDIO_SAMPLE_PERIOD_CYCLES: u32 = 3_500; // 168 MHz / 48 kHz
const AUDIO_STARTUP_LATENCY_CYCLES: u32 = 168_000; // 1 ms edge-queue lead
const AUDIO_AMPLITUDE: i16 = 12_000;

static AUDIO_ALT1_ACTIVE: AtomicBool = AtomicBool::new(false);
static AUDIO_ATTEMPT_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_SUBMIT_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_WRITE_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_COMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_INCOMPLETE_COUNT: AtomicU32 = AtomicU32::new(0);
static AUDIO_RETRY_STREAK: AtomicU32 = AtomicU32::new(0);

/// Snapshot of the UAC2 stream state and EP2 transfer counters.
pub fn diagnostics() -> (bool, u32, u32, u32, u32, u32, u32) {
    (
        AUDIO_ALT1_ACTIVE.load(Ordering::Acquire),
        AUDIO_ATTEMPT_COUNT.load(Ordering::Relaxed),
        AUDIO_SUBMIT_COUNT.load(Ordering::Relaxed),
        AUDIO_WRITE_FAIL_COUNT.load(Ordering::Relaxed),
        AUDIO_COMPLETE_COUNT.load(Ordering::Relaxed),
        AUDIO_INCOMPLETE_COUNT.load(Ordering::Relaxed),
        AUDIO_RETRY_STREAK.load(Ordering::Relaxed),
    )
}

unsafe extern "C" {
    fn bk_audio_start() -> u8;
    fn bk_audio_stop();
    fn bk_audio_pop_edge(cycles: *mut u32, level: *mut u8) -> u8;
}

/// Audio source used by ktims/usbd-uac2.
///
/// It is a fixed 48 kHz clock. The class allocates a High-Speed isochronous IN
/// endpoint with one extra sample of packet-size headroom; this handler emits
/// six mono 16-bit samples (12 bytes) per 125-us microframe.
pub struct BkAudio {
    next_sample_cycle: u32,
    sample_level: bool,
    pending_edge: Option<(u32, bool)>,
    active: bool,
}

impl BkAudio {
    pub const fn new() -> Self {
        Self {
            next_sample_cycle: 0,
            sample_level: false,
            pending_edge: None,
            active: false,
        }
    }

    #[inline(always)]
    fn cycle_counter() -> u32 {
        cortex_m::peripheral::DWT::cycle_count()
    }

    fn reset_diagnostics() {
        AUDIO_ATTEMPT_COUNT.store(0, Ordering::Relaxed);
        AUDIO_SUBMIT_COUNT.store(0, Ordering::Relaxed);
        AUDIO_WRITE_FAIL_COUNT.store(0, Ordering::Relaxed);
        AUDIO_COMPLETE_COUNT.store(0, Ordering::Relaxed);
        AUDIO_INCOMPLETE_COUNT.store(0, Ordering::Relaxed);
        AUDIO_RETRY_STREAK.store(0, Ordering::Relaxed);
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
            // Signed wrapping subtraction is valid because the queue backlog
            // is far below half the 32-bit DWT counter period.
            if sample_cycle.wrapping_sub(edge_cycle) as i32 >= 0 {
                self.sample_level = edge_level;
                self.pending_edge = None;
            } else {
                break;
            }
        }
    }

    fn fill_packet(&mut self, packet: &mut [u8; AUDIO_PACKET_BYTES]) {
        for sample_index in 0..AUDIO_SAMPLES_PER_MICROFRAME {
            let sample_cycle = self.next_sample_cycle;
            self.apply_edges_through(sample_cycle);

            // Convert the BK's one-bit speaker signal to a centered PCM square
            // wave. This preserves transitions and avoids a DC-only 0/32767
            // representation at the USB capture interface.
            let sample = if self.sample_level {
                AUDIO_AMPLITUDE
            } else {
                -AUDIO_AMPLITUDE
            };
            let offset = sample_index * 2;
            packet[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());

            self.next_sample_cycle =
                self.next_sample_cycle.wrapping_add(AUDIO_SAMPLE_PERIOD_CYCLES);
        }
    }

    fn start(&mut self) {
        self.sample_level = unsafe { bk_audio_start() != 0 };
        self.pending_edge = None;
        self.next_sample_cycle =
            Self::cycle_counter().wrapping_sub(AUDIO_STARTUP_LATENCY_CYCLES);
        self.active = true;
        AUDIO_ALT1_ACTIVE.store(true, Ordering::Release);
        Self::reset_diagnostics();
    }

    fn stop(&mut self) {
        self.active = false;
        AUDIO_ALT1_ACTIVE.store(false, Ordering::Release);
        self.pending_edge = None;
        unsafe { bk_audio_stop() };
    }
}

impl Default for BkAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockSource for BkAudio {
    const CLOCK_TYPE: ClockType = ClockType::InternalFixed;
    const SOF_SYNC: bool = false;

    fn sample_rate(&self) -> u32 {
        AUDIO_SAMPLE_RATE
    }

    fn sample_rates(
        &self,
    ) -> core::result::Result<&[RangeEntry<u32>], UsbAudioClassError> {
        const RATES: [RangeEntry<u32>; 1] = [RangeEntry::new_fixed(AUDIO_SAMPLE_RATE)];
        Ok(&RATES)
    }

    fn clock_validity(
        &self,
    ) -> core::result::Result<bool, UsbAudioClassError> {
        Ok(true)
    }
}

impl<'a, B: UsbBus> AudioHandler<'a, B> for BkAudio {
    fn audio_data_rx(&mut self, _ep: &Endpoint<'a, B, Out>) {
        // This device exposes capture only; there is no USB playback stream.
    }

    fn audio_data_tx(&mut self, ep: &Endpoint<'a, B, In>) {
        if !self.active {
            return;
        }

        AUDIO_ATTEMPT_COUNT.fetch_add(1, Ordering::Relaxed);

        let mut packet = [0u8; AUDIO_PACKET_BYTES];
        self.fill_packet(&mut packet);

        // Use the project's FIFO-backed isochronous path. This avoids making
        // the UAC2 class depend on the DWC2 internal-DMA staging buffer.
        match ep.write_iso_batch_fifo(&packet, 1, 1) {
            Ok(_) => {
                AUDIO_SUBMIT_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                AUDIO_WRITE_FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn audio_data_tx_complete(&mut self) {
        AUDIO_COMPLETE_COUNT.fetch_add(1, Ordering::Relaxed);
        AUDIO_RETRY_STREAK.store(0, Ordering::Relaxed);
    }

    fn audio_data_tx_incomplete(&mut self) {
        AUDIO_INCOMPLETE_COUNT.fetch_add(1, Ordering::Relaxed);
        AUDIO_RETRY_STREAK.fetch_add(1, Ordering::Relaxed);
    }

    fn feedback(&mut self, _nominal: UsbIsochronousFeedback) -> Option<UsbIsochronousFeedback> {
        // An IN-only capture stream has no OUT feedback endpoint.
        None
    }

    fn alternate_setting_changed(&mut self, terminal: UsbDirection, alt_setting: u8) {
        if terminal != UsbDirection::In {
            return;
        }

        if alt_setting == 1 {
            self.start();
        } else {
            self.stop();
        }
    }
}
