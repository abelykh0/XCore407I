//! XCore407I board pin mux for the USB3300 ULPI PHY.

use stm32f4xx_hal::{
    gpio::{GpioExt, Speed},
    pac,
};

/// CMSIS clock symbol consumed by the STM32 DWC2 USB driver.
#[unsafe(no_mangle)]
pub static SystemCoreClock: u32 = 168_000_000;

/// Configure the XCore407I clock tree to 168 MHz from its 8 MHz HSE crystal.
pub fn configure_system_clock_168mhz() {
    const RCC_CR: *mut u32 = 0x4002_3800 as *mut u32;
    const RCC_PLLCFGR: *mut u32 = 0x4002_3804 as *mut u32;
    const RCC_CFGR: *mut u32 = 0x4002_3808 as *mut u32;
    const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
    const PWR_CR: *mut u32 = 0x4000_7000 as *mut u32;
    const FLASH_ACR: *mut u32 = 0x4002_3c00 as *mut u32;

    unsafe {
        RCC_APB1ENR.write_volatile(RCC_APB1ENR.read_volatile() | (1 << 28));
        PWR_CR.write_volatile((PWR_CR.read_volatile() & !(3 << 14)) | (3 << 14));
        FLASH_ACR.write_volatile((1 << 10) | (1 << 9) | (1 << 8) | 5);
        RCC_CR.write_volatile(RCC_CR.read_volatile() | (1 << 16));
        while RCC_CR.read_volatile() & (1 << 17) == 0 { core::hint::spin_loop(); }
        RCC_PLLCFGR.write_volatile(4 | (168 << 6) | (1 << 22) | (7 << 24));
        RCC_CR.write_volatile(RCC_CR.read_volatile() | (1 << 24));
        while RCC_CR.read_volatile() & (1 << 25) == 0 { core::hint::spin_loop(); }
        let cfgr = RCC_CFGR.read_volatile();
        let cfgr = (cfgr & !((0xf << 4) | (7 << 10) | (7 << 13) | 3))
            | (5 << 10) | (4 << 13);
        RCC_CFGR.write_volatile(cfgr | 2);
        while RCC_CFGR.read_volatile() & (3 << 2) != (2 << 2) { core::hint::spin_loop(); }
    }
}

/// Configure the external USB3300 ULPI signals for OTG_HS.
///
/// This must run before the OTG_HS bus is constructed. The board uses the
/// 8 MHz HSE crystal and a 168 MHz system clock; the OTG_HS peripheral clock
/// and external ULPI PHY provide the required 60 MHz ULPI interface clock.
pub fn configure_ulpi(
    rcc: &mut pac::RCC,
    gpioa: pac::GPIOA,
    gpiob: pac::GPIOB,
    gpioc: pac::GPIOC,
    gpioh: pac::GPIOH,
    gpioi: pac::GPIOI,
) {
    let a = gpioa.split(rcc);
    let b = gpiob.split(rcc);
    let c = gpioc.split(rcc);
    let h = gpioh.split(rcc);
    let i = gpioi.split(rcc);

    let _ulpi_pins = (
        a.pa3.into_alternate::<10>().speed(Speed::VeryHigh),  // D0
        a.pa5.into_alternate::<10>().speed(Speed::VeryHigh),  // CLK
        a.pa4.into_alternate::<12>().speed(Speed::VeryHigh),  // SOF
        b.pb0.into_alternate::<10>().speed(Speed::VeryHigh),  // D1
        b.pb1.into_alternate::<10>().speed(Speed::VeryHigh),  // D2
        b.pb10.into_alternate::<10>().speed(Speed::VeryHigh), // D3
        b.pb11.into_alternate::<10>().speed(Speed::VeryHigh), // D4
        b.pb12.into_alternate::<10>().speed(Speed::VeryHigh), // D5
        b.pb13.into_alternate::<10>().speed(Speed::VeryHigh), // D6
        b.pb5.into_alternate::<10>().speed(Speed::VeryHigh),  // D7
        c.pc0.into_alternate::<10>().speed(Speed::VeryHigh),  // STP
        h.ph4.into_alternate::<10>().speed(Speed::VeryHigh),  // NXT
        i.pi11.into_alternate::<10>().speed(Speed::VeryHigh), // DIR
    );
}
