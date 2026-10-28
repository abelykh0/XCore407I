//! STM32F407 RTC initialization and cached 24-hour clock display.

use core::sync::atomic::{AtomicU32, Ordering};

const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
const RCC_BDCR: *mut u32 = 0x4002_3870 as *mut u32;
const PWR_CR: *mut u32 = 0x4000_7000 as *mut u32;

const RTC_BASE: usize = 0x4000_2800;
const RTC_TR: *mut u32 = (RTC_BASE + 0x00) as *mut u32;
const RTC_DR: *mut u32 = (RTC_BASE + 0x04) as *mut u32;
const RTC_CR: *mut u32 = (RTC_BASE + 0x08) as *mut u32;
const RTC_ISR: *mut u32 = (RTC_BASE + 0x0c) as *mut u32;
const RTC_PRER: *mut u32 = (RTC_BASE + 0x10) as *mut u32;
const RTC_WPR: *mut u32 = (RTC_BASE + 0x24) as *mut u32;

const PWR_CLOCK_ENABLE: u32 = 1 << 28;
const PWR_BACKUP_WRITE_ENABLE: u32 = 1 << 8;
const RTC_LSE_ENABLE: u32 = 1;
const RTC_LSE_READY: u32 = 1 << 1;
const RTC_CLOCK_SELECT_MASK: u32 = 0b11 << 8;
const RTC_CLOCK_SELECT_LSE: u32 = 0b01 << 8;
const RTC_CLOCK_ENABLE: u32 = 1 << 15;
const RTC_INIT: u32 = 1 << 7;
const RTC_INIT_READY: u32 = 1 << 6;
const RTC_SHADOW_SYNC: u32 = 1 << 5;
const RTC_CALENDAR_INITIALIZED: u32 = 1 << 4;
const RTC_24_HOUR_FORMAT: u32 = 1 << 6;

static DISPLAY_HHMMSS: AtomicU32 = AtomicU32::new(0);

/// Start the LSE-backed RTC and initialize it to 00:00:00 only if its calendar
/// has never been initialized. Existing backup-domain time is preserved.
pub fn initialize() -> bool {
    modify(RCC_APB1ENR, |value| value | PWR_CLOCK_ENABLE);
    modify(PWR_CR, |value| value | PWR_BACKUP_WRITE_ENABLE);

    if read(RTC_ISR) & RTC_CALENDAR_INITIALIZED == 0 {
        let mut bdcr = read(RCC_BDCR);
        let clock_source = bdcr & RTC_CLOCK_SELECT_MASK;
        if clock_source != 0 && clock_source != RTC_CLOCK_SELECT_LSE {
            return false;
        }

        bdcr |= RTC_LSE_ENABLE;
        write(RCC_BDCR, bdcr);
        let mut timeout = 100_000_000u32;
        while read(RCC_BDCR) & RTC_LSE_READY == 0 && timeout != 0 {
            timeout -= 1;
        }
        if read(RCC_BDCR) & RTC_LSE_READY == 0 {
            return false;
        }

        bdcr = read(RCC_BDCR);
        bdcr = (bdcr & !RTC_CLOCK_SELECT_MASK) | RTC_CLOCK_SELECT_LSE | RTC_CLOCK_ENABLE;
        write(RCC_BDCR, bdcr);

        // Unlock the RTC write-protected registers and enter initialization mode.
        write(RTC_WPR, 0xca);
        write(RTC_WPR, 0x53);
        modify(RTC_ISR, |value| value | RTC_INIT);
        let mut timeout = 1_000_000u32;
        while read(RTC_ISR) & RTC_INIT_READY == 0 && timeout != 0 {
            timeout -= 1;
        }
        if read(RTC_ISR) & RTC_INIT_READY == 0 {
            write(RTC_WPR, 0xff);
            return false;
        }

        write(RTC_PRER, (127 << 16) | 255);
        modify(RTC_CR, |value| value & !RTC_24_HOUR_FORMAT);
        // Match the C reference's first-use clock setting, midnight.
        write(RTC_TR, 0);
        // 2000-01-01 is Saturday (RTC weekday encoding: Monday = 1).
        write(RTC_DR, (6 << 13) | (1 << 8) | 1);
        modify(RTC_ISR, |value| value & !RTC_INIT);
        write(RTC_WPR, 0xff);
    }

    // On a CPU reset the RTC shadow registers may need resynchronization even
    // though the backup-powered calendar itself kept running.
    modify(RTC_ISR, |value| value & !RTC_SHADOW_SYNC);
    let mut timeout = 1_000_000u32;
    while read(RTC_ISR) & RTC_SHADOW_SYNC == 0 && timeout != 0 {
        timeout -= 1;
    }
    if read(RTC_ISR) & RTC_SHADOW_SYNC == 0 {
        return false;
    }

    refresh_display_time();
    true
}

/// Refresh the overlay's cached time. Call this about once per second.
pub fn refresh_display_time() {
    let time = read(RTC_TR);
    // Reading the date after the time completes the RTC shadow-register
    // sequence and keeps subsequent reads synchronized to a calendar second.
    let _date = read(RTC_DR);

    let Some(hours) = decode_bcd((time >> 16) & 0x3f) else {
        return;
    };
    let Some(minutes) = decode_bcd((time >> 8) & 0x7f) else {
        return;
    };
    let Some(seconds) = decode_bcd(time & 0x7f) else {
        return;
    };
    if hours >= 24 || minutes >= 60 || seconds >= 60 {
        return;
    }

    DISPLAY_HHMMSS.store(
        (hours as u32) * 10_000 + (minutes as u32) * 100 + seconds as u32,
        Ordering::Relaxed,
    );
}

/// Return the cached time with one blank character on each side.
pub(crate) fn display_time() -> [u8; 10] {
    let time = DISPLAY_HHMMSS.load(Ordering::Relaxed);
    let hours = (time / 10_000) as u8;
    let minutes = ((time / 100) % 100) as u8;
    let seconds = (time % 100) as u8;
    [
        b' ',
        b'0' + hours / 10,
        b'0' + hours % 10,
        b':',
        b'0' + minutes / 10,
        b'0' + minutes % 10,
        b':',
        b'0' + seconds / 10,
        b'0' + seconds % 10,
        b' ',
    ]
}

fn decode_bcd(value: u32) -> Option<u8> {
    let units = (value & 0x0f) as u8;
    let tens = ((value >> 4) & 0x0f) as u8;
    if units > 9 || tens > 9 {
        return None;
    }
    Some(tens * 10 + units)
}

#[inline(always)]
fn read(address: *mut u32) -> u32 {
    unsafe { address.read_volatile() }
}

#[inline(always)]
fn write(address: *mut u32, value: u32) {
    unsafe { address.write_volatile(value) }
}

#[inline(always)]
fn modify(address: *mut u32, f: impl FnOnce(u32) -> u32) {
    write(address, f(read(address)));
}
