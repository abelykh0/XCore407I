#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::{entry, exception};
use panic_halt as _;
use synopsys_usb_otg::UsbBus;
use usb_device::device::{UsbDevice, UsbDeviceBuilder, UsbVidPid};
use xcore407i_blinker::usb_uvc::{configure_ulpi_pins, usb_bus, F407UsbHs, UvcUsbClass};

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4002_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4002_0020 as *mut u32;
const GPIOC_MODER: *mut u32 = 0x4002_0800 as *mut u32;
const GPIOC_AFRL: *mut u32 = 0x4002_0820 as *mut u32;
// STM32F407 Ethernet MAC register offsets: MACMIIAR=0x10, MACMIIDR=0x14.
const ETH_MACMIIAR: *mut u32 = 0x4002_8010 as *mut u32;
const ETH_MACMIIDR: *mut u32 = 0x4002_8014 as *mut u32;

const PHY_ADDRESS: u32 = 1;
const PHY_LED_CONTROL: u8 = 0x18;
const MII_BUSY: u32 = 1 << 0;
const MII_WRITE: u32 = 1 << 1;
// HCLK=168 MHz: STM32F4 MAC MDC divisor /102 (CR=100b).
const MII_CLOCK_DIV102: u32 = 4 << 2;
const BLINK_INTERVAL_MS: u32 = 500;
static TICKS: AtomicU32 = AtomicU32::new(0);

#[entry]
fn main() -> ! {
    system_clock_168mhz();
    configure_blinker_pins();
    unsafe {
        ETH_MACMIIAR.write_volatile(MII_CLOCK_DIV102);
        phy_write(PHY_LED_CONTROL, 0x38); // manual mode; both LEDs controllable
    }

    let mut core = cortex_m::Peripherals::take().unwrap();
    core.SYST.set_clock_source(SystClkSource::Core);
    core.SYST.set_reload(168_000 - 1); // 1 ms at 168 MHz
    core.SYST.clear_current();
    core.SYST.enable_interrupt();
    core.SYST.enable_counter();

    unsafe { configure_ulpi_pins() };
    let bus = usb_bus();
    let mut camera = UvcUsbClass::new(&bus);
    let mut usb: UsbDevice<'_, UsbBus<F407UsbHs>> = UsbDeviceBuilder::new(
        &bus,
        // Development VID/PID; USB class descriptors identify this as UVC.
        UsbVidPid(0x1209, 0x4071),
    )
    .composite_with_iads()
    .max_packet_size_0(64)
    .unwrap()
    .build();

    let mut last_blink = 0;
    loop {
        // Poll continuously: control transfers and each HS isochronous IN
        // completion must be serviced promptly while the camera is streaming.
        let _ = usb.poll(&mut [&mut camera]);

        let now = TICKS.load(Ordering::Relaxed);
        if now.wrapping_sub(last_blink) >= BLINK_INTERVAL_MS {
            last_blink = now;
            unsafe {
                let mut led = phy_read(PHY_LED_CONTROL);
                led ^= 0x03;
                phy_write(PHY_LED_CONTROL, led);
            }
        }
    }
}

#[exception]
fn SysTick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

fn system_clock_168mhz() {
    const RCC_CR: *mut u32 = 0x4002_3800 as *mut u32;
    const RCC_PLLCFGR: *mut u32 = 0x4002_3804 as *mut u32;
    const RCC_CFGR: *mut u32 = 0x4002_3808 as *mut u32;
    const PWR_CR: *mut u32 = 0x4000_7000 as *mut u32;
    const FLASH_ACR: *mut u32 = 0x4002_3c00 as *mut u32;

    unsafe {
        // The board has the 8 MHz HSE crystal used by the original firmware.
        RCC_APB1ENR.write_volatile(RCC_APB1ENR.read_volatile() | (1 << 28)); // PWR clock
        PWR_CR.write_volatile((PWR_CR.read_volatile() & !(3 << 14)) | (3 << 14)); // scale 1
        FLASH_ACR.write_volatile((1 << 10) | (1 << 9) | (1 << 8) | 5); // caches + 5 WS

        RCC_CR.write_volatile(RCC_CR.read_volatile() | (1 << 16)); // HSEON
        while RCC_CR.read_volatile() & (1 << 17) == 0 {}

        // PLLM=4, PLLN=168, PLLP=/2, PLL source=HSE, PLLQ=7 => 168/48 MHz.
        RCC_PLLCFGR.write_volatile(4 | (168 << 6) | (1 << 22) | (7 << 24));
        RCC_CR.write_volatile(RCC_CR.read_volatile() | (1 << 24)); // PLLON
        while RCC_CR.read_volatile() & (1 << 25) == 0 {}

        // HCLK=/1, APB1=/4, APB2=/2, then switch SYSCLK to PLL.
        let cfgr = RCC_CFGR.read_volatile();
        let cfgr = (cfgr & !((0xf << 4) | (7 << 10) | (7 << 13) | 3)) | (5 << 10) | (4 << 13);
        RCC_CFGR.write_volatile(cfgr | 2);
        while RCC_CFGR.read_volatile() & (3 << 2) != (2 << 2) {}
    }
}

fn configure_blinker_pins() {
    unsafe {
        // Ethernet MAC, GPIOA and GPIOC clocks.
        RCC_AHB1ENR.write_volatile(RCC_AHB1ENR.read_volatile() | (1 << 0) | (1 << 2) | (1 << 25));
        // PA2 = ETH_MDIO, PC1 = ETH_MDC (alternate function 11).
        GPIOA_MODER
            .write_volatile((GPIOA_MODER.read_volatile() & !(3 << (2 * 2))) | (2 << (2 * 2)));
        GPIOA_AFRL
            .write_volatile((GPIOA_AFRL.read_volatile() & !(0xf << (2 * 4))) | (11 << (2 * 4)));
        GPIOC_MODER.write_volatile((GPIOC_MODER.read_volatile() & !(3 << 2)) | (2 << 2));
        GPIOC_AFRL.write_volatile((GPIOC_AFRL.read_volatile() & !(0xf << 4)) | (11 << 4));
    }
}

unsafe fn phy_read(reg: u8) -> u16 {
    while ETH_MACMIIAR.read_volatile() & MII_BUSY != 0 {}
    ETH_MACMIIAR
        .write_volatile((PHY_ADDRESS << 11) | ((reg as u32) << 6) | MII_CLOCK_DIV102 | MII_BUSY);
    while ETH_MACMIIAR.read_volatile() & MII_BUSY != 0 {}
    ETH_MACMIIDR.read_volatile() as u16
}

unsafe fn phy_write(reg: u8, value: u16) {
    while ETH_MACMIIAR.read_volatile() & MII_BUSY != 0 {}
    ETH_MACMIIDR.write_volatile(value as u32);
    ETH_MACMIIAR.write_volatile(
        (PHY_ADDRESS << 11) | ((reg as u32) << 6) | MII_CLOCK_DIV102 | MII_WRITE | MII_BUSY,
    );
    while ETH_MACMIIAR.read_volatile() & MII_BUSY != 0 {}
}
