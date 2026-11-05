//! Deterministic UAC2 capture test source: a precomputed 1 kHz PCM16 sine wave.
//!
//! This deliberately removes the BK emulator edge queue, DWT timing, and
//! runtime sample conversion from the audio path. Each HS microframe submits
//! exactly six mono samples (12 bytes) at 48 kHz.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use usb_device::bus::UsbBus;
use usb_device::endpoint::{Endpoint, In, Out};
use usb_device::UsbDirection;
use usbd_uac2::descriptors::ClockType;
use usbd_uac2::{
    AudioHandler, ClockSource, RangeEntry, UsbAudioClassError, UsbIsochronousFeedback,
};

const AUDIO_SAMPLE_RATE: u32 = 48_000;
const SAMPLES_PER_MICROFRAME: usize = 6;
const PACKET_BYTES: usize = SAMPLES_PER_MICROFRAME * 2;

// Small audio packet buffer. FIFO submission copies these bytes into the OTG TX FIFO.

// One 1 ms cycle: 48 samples at 48 kHz, producing a 1 kHz sine wave
// at a conservative amplitude of +/-8000 PCM16.
const TEST_TONE: [i16; 48] = [
    0, 1044, 2071, 3061, 4000, 4870, 5657, 6347,
    6928, 7391, 7727, 7932, 8000, 7932, 7727, 7391,
    6928, 6347, 5657, 4870, 4000, 3061, 2071, 1044,
    0, -1044, -2071, -3061, -4000, -4870, -5657, -6347,
    -6928, -7391, -7727, -7932, -8000, -7932, -7727, -7391,
    -6928, -6347, -5657, -4870, -4000, -3061, -2071, -1044,
];

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

struct AudioPacket([u8; PACKET_BYTES]);

pub struct TestToneAudio {
    sample_index: usize,
    active: bool,
    transfer_in_flight: bool,
    packet: AudioPacket,
}

impl TestToneAudio {
    const RATES: [RangeEntry<u32>; 1] = [RangeEntry::new_fixed(AUDIO_SAMPLE_RATE)];

    pub const fn new() -> Self {
        Self {
            sample_index: 0,
            active: false,
            transfer_in_flight: false,
            packet: AudioPacket([0; PACKET_BYTES]),
        }
    }

    fn reset_diagnostics() {
        AUDIO_ATTEMPT_COUNT.store(0, Ordering::Relaxed);
        AUDIO_SUBMIT_COUNT.store(0, Ordering::Relaxed);
        AUDIO_WRITE_FAIL_COUNT.store(0, Ordering::Relaxed);
        AUDIO_COMPLETE_COUNT.store(0, Ordering::Relaxed);
        AUDIO_INCOMPLETE_COUNT.store(0, Ordering::Relaxed);
        AUDIO_RETRY_STREAK.store(0, Ordering::Relaxed);
    }
}

impl Default for TestToneAudio {
    fn default() -> Self { Self::new() }
}

impl ClockSource for TestToneAudio {
    const CLOCK_TYPE: ClockType = ClockType::InternalFixed;
    const SOF_SYNC: bool = false;

    fn sample_rate(&self) -> u32 { AUDIO_SAMPLE_RATE }

    fn sample_rates(&self) -> core::result::Result<&[RangeEntry<u32>], UsbAudioClassError> {
        Ok(&Self::RATES)
    }

    fn clock_validity(&self) -> core::result::Result<bool, UsbAudioClassError> {
        Ok(true)
    }
}

impl<'a, B: UsbBus> AudioHandler<'a, B> for TestToneAudio {
    fn audio_data_rx(&mut self, _ep: &Endpoint<'a, B, Out>) {
        // Capture-only test: there is no USB playback stream.
    }

    fn audio_data_tx(&mut self, ep: &Endpoint<'a, B, In>) {
        if !self.active { return; }

        if self.transfer_in_flight {
            return;
        }

        AUDIO_ATTEMPT_COUNT.fetch_add(1, Ordering::Relaxed);
        let packet = &mut self.packet.0;
        for sample_offset in 0..SAMPLES_PER_MICROFRAME {
            let sample = TEST_TONE[(self.sample_index + sample_offset) % TEST_TONE.len()];
            let offset = sample_offset * 2;
            packet[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
        }

        // FIFO submission copies these 12 bytes into the OTG transmit FIFO.
        match ep.write_iso_batch_fifo(&packet[..], 1, 1) {
            Ok(_) => {
                self.transfer_in_flight = true;
                self.sample_index = (self.sample_index + SAMPLES_PER_MICROFRAME) % TEST_TONE.len();
                AUDIO_SUBMIT_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                AUDIO_WRITE_FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn audio_data_tx_complete(&mut self) {
        self.transfer_in_flight = false;
        AUDIO_COMPLETE_COUNT.fetch_add(1, Ordering::Relaxed);
        AUDIO_RETRY_STREAK.store(0, Ordering::Relaxed);
    }

    fn audio_data_tx_incomplete(&mut self) {
        self.transfer_in_flight = false;
        AUDIO_INCOMPLETE_COUNT.fetch_add(1, Ordering::Relaxed);
        AUDIO_RETRY_STREAK.fetch_add(1, Ordering::Relaxed);
    }

    fn feedback(&mut self, _nominal: UsbIsochronousFeedback) -> Option<UsbIsochronousFeedback> {
        None
    }

    fn alternate_setting_changed(&mut self, terminal: UsbDirection, alt_setting: u8) {
        if terminal != UsbDirection::In { return; }

        if alt_setting == 1 {
            self.sample_index = 0;
            self.transfer_in_flight = false;
            self.active = true;
            AUDIO_ALT1_ACTIVE.store(true, Ordering::Release);
            Self::reset_diagnostics();
        } else {
            self.active = false;
            self.transfer_in_flight = false;
            AUDIO_ALT1_ACTIVE.store(false, Ordering::Release);
        }
    }
}
