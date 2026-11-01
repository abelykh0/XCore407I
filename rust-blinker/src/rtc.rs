//! STM32F407 RTC initialization and cached 24-hour clock display.
//!
//! Register access goes through the STM32F407 PAC. The RTC remains backed by
//! LSE and the backup-domain calendar is only initialized on first use.

use core::sync::atomic::{AtomicU32, Ordering};
use stm32f4::stm32f407::{PWR, RCC, RTC};

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
    // The PWR peripheral must be clocked before DBP can be changed.
    let rcc = unsafe { &*RCC::ptr() };
    let pwr = unsafe { &*PWR::ptr() };
    let rtc = unsafe { &*RTC::ptr() };

    rcc.apb1enr().modify(|r, w| unsafe {
        w.bits(r.bits() | PWR_CLOCK_ENABLE)
    });
    pwr.cr().modify(|r, w| unsafe {
        w.bits(r.bits() | PWR_BACKUP_WRITE_ENABLE)
    });

    if rtc.isr().read().bits() & RTC_CALENDAR_INITIALIZED == 0 {
        let mut bdcr = rcc.bdcr().read().bits();
        let clock_source = bdcr & RTC_CLOCK_SELECT_MASK;
        if clock_source != 0 && clock_source != RTC_CLOCK_SELECT_LSE {
            return false;
        }

        bdcr |= RTC_LSE_ENABLE;
        rcc.bdcr().write(|w| unsafe { w.bits(bdcr) });

        let mut timeout = 100_000_000u32;
        while rcc.bdcr().read().bits() & RTC_LSE_READY == 0 && timeout != 0 {
            timeout -= 1;
        }
        if rcc.bdcr().read().bits() & RTC_LSE_READY == 0 {
            return false;
        }

        bdcr = rcc.bdcr().read().bits();
        bdcr = (bdcr & !RTC_CLOCK_SELECT_MASK)
            | RTC_CLOCK_SELECT_LSE
            | RTC_CLOCK_ENABLE;
        rcc.bdcr().write(|w| unsafe { w.bits(bdcr) });

        // Unlock the RTC write-protected registers and enter initialization mode.
        rtc.wpr().write(|w| unsafe { w.bits(0xca) });
        rtc.wpr().write(|w| unsafe { w.bits(0x53) });
        rtc.isr().modify(|r, w| unsafe {
            w.bits(r.bits() | RTC_INIT)
        });

        let mut timeout = 1_000_000u32;
        while rtc.isr().read().bits() & RTC_INIT_READY == 0 && timeout != 0 {
            timeout -= 1;
        }
        if rtc.isr().read().bits() & RTC_INIT_READY == 0 {
            rtc.wpr().write(|w| unsafe { w.bits(0xff) });
            return false;
        }

        rtc.prer().write(|w| unsafe { w.bits((127 << 16) | 255) });
        rtc.cr().modify(|r, w| unsafe {
            w.bits(r.bits() & !RTC_24_HOUR_FORMAT)
        });
        // Match the C reference's first-use clock setting, midnight.
        rtc.tr().write(|w| unsafe { w.bits(0) });
        // 2000-01-01 is Saturday (RTC weekday encoding: Monday = 1).
        rtc.dr().write(|w| unsafe { w.bits((6 << 13) | (1 << 8) | 1) });
        rtc.isr().modify(|r, w| unsafe {
            w.bits(r.bits() & !RTC_INIT)
        });
        rtc.wpr().write(|w| unsafe { w.bits(0xff) });
    }

    // On a CPU reset the RTC shadow registers may need resynchronization even
    // though the backup-powered calendar itself kept running.
    rtc.isr().modify(|r, w| unsafe {
        w.bits(r.bits() & !RTC_SHADOW_SYNC)
    });
    let mut timeout = 1_000_000u32;
    while rtc.isr().read().bits() & RTC_SHADOW_SYNC == 0 && timeout != 0 {
        timeout -= 1;
    }
    if rtc.isr().read().bits() & RTC_SHADOW_SYNC == 0 {
        return false;
    }

    refresh_display_time();
    true
}

/// Refresh the overlay's cached time. Call this about once per second.
pub fn refresh_display_time() {
    let rtc = unsafe { &*RTC::ptr() };
    let time = rtc.tr().read().bits();
    // Reading the date after the time completes the RTC shadow-register
    // sequence and keeps subsequent reads synchronized to a calendar second.
    let _date = rtc.dr().read().bits();

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
