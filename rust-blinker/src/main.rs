#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::{entry, exception};
use panic_halt as _;
use stm32f4::stm32f407::interrupt;
use synopsys_usb_otg::UsbBus;
use usb_device::device::{UsbDevice, UsbDeviceBuilder, UsbVidPid};
use xcore407i_blinker::camera::{clear_demo_canvas, update_clock_from_rtc};
use xcore407i_blinker::rtc;
use xcore407i_blinker::usb_uvc::{
    configure_ulpi_pins, usb_bus, F407UsbHs, UvcUsbClass,
};
use xcore407i_blinker::rotozoom_demo;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4002_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4002_0020 as *mut u32;
const GPIOC_MODER: *mut u32 = 0x4002_0800 as *mut u32;
const GPIOC_AFRL: *mut u32 = 0x4002_0820 as *mut u32;
// STM32F407 Ethernet MAC register offsets: MACMIIAR=0x10, MACMIIDR=0x14.
const ETH_MACMIIAR: *mut u32 = 0x4002_8010 as *mut u32;
const ETH_MACMIIDR: *mut u32 = 0x4002_8014 as *mut u32;

// TIM2 is clocked from the APB1 timer clock. With PPRE1=/4, PCLK1=42 MHz
// and the STM32F4 timer-clock multiplier makes TIM2CLK=84 MHz.
const TIM2_BASE: usize = 0x4000_0000;
const TIM2_CR1: *mut u32 = (TIM2_BASE + 0x00) as *mut u32;
const TIM2_DIER: *mut u32 = (TIM2_BASE + 0x0C) as *mut u32;
const TIM2_SR: *mut u32 = (TIM2_BASE + 0x10) as *mut u32;
const TIM2_EGR: *mut u32 = (TIM2_BASE + 0x14) as *mut u32;
const TIM2_CNT: *mut u32 = (TIM2_BASE + 0x24) as *mut u32;
const TIM2_PSC: *mut u32 = (TIM2_BASE + 0x28) as *mut u32;
const TIM2_ARR: *mut u32 = (TIM2_BASE + 0x2C) as *mut u32;

const PHY_ADDRESS: u32 = 1;
const PHY_LED_CONTROL: u8 = 0x18;
const MII_BUSY: u32 = 1 << 0;
const MII_WRITE: u32 = 1 << 1;
// HCLK=168 MHz: STM32F4 MAC MDC divisor /102 (CR=100b).
const MII_CLOCK_DIV102: u32 = 4 << 2;
const BLINK_INTERVAL_MS: u32 = 500;

// One timer interrupt per USB HS microframe (125 us, 8 kHz).
const USB_POLL_TIMER_HZ: u32 = 8_000;
const TIM2_IRQ_NUMBER: usize = 28;
const OTG_HS_IRQ_NUMBER: usize = 77;
const TIM2_PRIORITY: u8 = 2;
const SYSTICK_PRIORITY: u8 = 1;

static TICKS: AtomicU32 = AtomicU32::new(0);

const CPU_SAMPLE_INTERVAL_MS: u32 = 1000;
const CORE_CLOCK_HZ: u64 = 168_000_000;
static LAST_CPU_SAMPLE: AtomicU32 = AtomicU32::new(0);

type BoardUsbBus = UsbBus<F407UsbHs>;
type BoardUsbDevice = UsbDevice<'static, BoardUsbBus>;
type BoardUvcClass = UvcUsbClass<'static, BoardUsbBus>;

// These pointers are published once, before TIM2 is enabled. After that,
// TIM2 is the sole owner of both mutable objects. main() must not access them.
static USB_DEVICE_PTR: AtomicPtr<BoardUsbDevice> = AtomicPtr::new(ptr::null_mut());
static CAMERA_PTR: AtomicPtr<BoardUvcClass> = AtomicPtr::new(ptr::null_mut());

#[entry]
fn main() -> ! {
    system_clock_168mhz();
    enable_fpu();
    let rtc_ready = rtc::initialize();
    update_clock_from_rtc();

    enable_cycle_counter();
    //initialize_demo_canvas();
    clear_demo_canvas(0x10);

    configure_blinker_pins();
    unsafe {
        ETH_MACMIIAR.write_volatile(MII_CLOCK_DIV102);
        phy_write(PHY_LED_CONTROL, 0x38); // manual mode; both LEDs controllable
    }

    let mut core = cortex_m::Peripherals::take().unwrap();

    // Keep the 1 ms application timebase. It is independent of USB polling.
    core.SYST.set_clock_source(SystClkSource::Core);
    core.SYST.set_reload(168_000 - 1); // 1 ms at 168 MHz
    core.SYST.clear_current();
    configure_exception_priorities();
    core.SYST.enable_interrupt();
    core.SYST.enable_counter();

    unsafe { configure_ulpi_pins() };
    let bus = usb_bus().expect("USB bus initialization failed");

    // The large endpoint buffers remain in their existing DMA-visible SRAM
    // allocation. The UsbDevice itself remains in CCM as in the original code.
    static mut CAMERA: MaybeUninit<BoardUvcClass> = MaybeUninit::uninit();

    #[link_section = ".ccmram.usb_device"]
    static mut USB_DEVICE: MaybeUninit<BoardUsbDevice> = MaybeUninit::uninit();

    // Construct in place and publish raw pointers. No long-lived &mut reference
    // is kept in main; the TIM2 ISR creates the only mutable references.
    let camera_ptr = unsafe { (*ptr::addr_of_mut!(CAMERA)).as_mut_ptr() };

    unsafe {
        camera_ptr.write(
            UvcUsbClass::new(bus).expect("UVC USB class initialization failed")
        );
    }

    let usb_ptr = unsafe { (*ptr::addr_of_mut!(USB_DEVICE)).as_mut_ptr() };
    unsafe {
        usb_ptr.write(
            UsbDeviceBuilder::new(
                bus,
                // Development VID/PID; USB class descriptors identify this as UVC.
                UsbVidPid(0x1209, 0x4071),
            )
            .composite_with_iads()
            .max_packet_size_0(64)
            .unwrap()
            .build(),
        );
    }

    CAMERA_PTR.store(camera_ptr, Ordering::Release);
    USB_DEVICE_PTR.store(usb_ptr, Ordering::Release);

    // Start only after both objects are fully initialized and published.
    configure_usb_poll_timer();

    let mut last_blink = 0;
    let mut last_rtc_sample = 0;

    loop {
        // USB polling and camera-class access now belong exclusively to TIM2.
        let now = TICKS.load(Ordering::Relaxed);
        if now.wrapping_sub(last_blink) >= BLINK_INTERVAL_MS {
            last_blink = now;
            unsafe {
                let mut led = phy_read(PHY_LED_CONTROL);
                led ^= 0x03;
                phy_write(PHY_LED_CONTROL, led);
            }
        }

        if rtc_ready && now.wrapping_sub(last_rtc_sample) >= 1000 {
            last_rtc_sample = now;
            rtc::refresh_display_time();
            update_clock_from_rtc();
        }

        rotozoom_demo::render_next();
        //rotozoom_demo::render_next();
    }
}

/// TIM2 fires at 8 kHz. This is the only place that calls UsbDevice::poll().
#[interrupt]
fn TIM2() {
    unsafe {
        // TIM2 update interrupt flag is write-zero-to-clear.
        TIM2_SR.write_volatile(TIM2_SR.read_volatile() & !1);
    }

    let usb_ptr = USB_DEVICE_PTR.load(Ordering::Acquire);
    let camera_ptr = CAMERA_PTR.load(Ordering::Acquire);
    if usb_ptr.is_null() || camera_ptr.is_null() {
        return;
    }

    // SAFETY: pointers are initialized before TIM2 is enabled; this ISR is the
    // exclusive owner. main() never dereferences either pointer after publish.
    let usb = unsafe { &mut *usb_ptr };
    let camera = unsafe { &mut *camera_ptr };

    let _ = usb.poll(&mut [camera]);

    /*
    // CPU load reporting 
    let now = TICKS.load(Ordering::Relaxed);
    let last = LAST_CPU_SAMPLE.load(Ordering::Relaxed);
    if now.wrapping_sub(last) >= CPU_SAMPLE_INTERVAL_MS {
        LAST_CPU_SAMPLE.store(now, Ordering::Relaxed);

        let (busy, generate, submit) = take_uvc_cycle_metrics();
        let busy_cycles = busy as u64;
        let busy_percent = ((busy_cycles * 100) / CORE_CLOCK_HZ).min(100) as u8;
        let generate_percent = (((generate as u64) * 100) / CORE_CLOCK_HZ).min(100) as u8;
        let submit_percent = (((submit as u64) * 100) / CORE_CLOCK_HZ).min(100) as u8;
        camera.set_cpu_metrics(100 - busy_percent, generate_percent, submit_percent);
    }
    */
}

/// Set a simple preemption-priority grouping and the system-handler priorities.
/// STM32F407 implements four priority bits, stored in the upper nibble.
fn configure_exception_priorities() {
    const SCB_AIRCR: *mut u32 = 0xE000_ED0C as *mut u32;
    const SCB_SHPR3: *mut u32 = 0xE000_ED20 as *mut u32;

    unsafe {
        // PRIGROUP=0: all implemented priority bits are preemption priority.
        let aircr = SCB_AIRCR.read_volatile();
        SCB_AIRCR.write_volatile((aircr & !(7 << 8)) | (0x5FA << 16));

        // SysTick is priority 2 (byte 3 of SHPR3, upper nibble implemented).
        let shpr3 = SCB_SHPR3.read_volatile();
        SCB_SHPR3.write_volatile(
            (shpr3 & 0x00FF_FFFF) | ((SYSTICK_PRIORITY as u32) << 28),
        );

        // This Synopsys driver is polled; it does not provide an OTG_HS NVIC
        // handler. Keep OTG_HS disabled so the core's GINT output cannot vector
        // into the default handler. UsbBus::poll reads and services GINTSTS.
        let otg_word = OTG_HS_IRQ_NUMBER / 32;
        let otg_bit = 1u32 << (OTG_HS_IRQ_NUMBER % 32);
        let icer = (0xE000_E180 + otg_word * 4) as *mut u32;
        let icpr = (0xE000_E280 + otg_word * 4) as *mut u32;
        icer.write_volatile(otg_bit);
        icpr.write_volatile(otg_bit);
    }
}

/// Configure TIM2 for an exact 125 us update period and enable its NVIC line.
fn configure_usb_poll_timer() {
    const NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;
    const NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;
    const NVIC_IPR: *mut u8 = 0xE000_E400 as *mut u8;

    // TIM2CLK=84 MHz, PSC=0, ARR=10499 => 8,000 updates/second.
    let timer_ticks = 84_000_000 / USB_POLL_TIMER_HZ;
    unsafe {
        // Enable TIM2 on APB1.
        RCC_APB1ENR.write_volatile(RCC_APB1ENR.read_volatile() | 1);
        let _ = RCC_APB1ENR.read_volatile();

        TIM2_CR1.write_volatile(0);
        TIM2_DIER.write_volatile(0);
        TIM2_PSC.write_volatile(0);
        TIM2_ARR.write_volatile(timer_ticks - 1);
        TIM2_CNT.write_volatile(0);
        TIM2_EGR.write_volatile(1); // latch PSC/ARR immediately
        TIM2_SR.write_volatile(0); // clear update flag from UG

        // Four priority bits on STM32F407; priority 1 = 0x10.
        NVIC_IPR.add(TIM2_IRQ_NUMBER).write_volatile(TIM2_PRIORITY << 4);
        NVIC_ICPR0.write_volatile(1 << TIM2_IRQ_NUMBER);

        // Update interrupt only. Start timer after enabling the NVIC line.
        TIM2_DIER.write_volatile(1);
        NVIC_ISER0.write_volatile(1 << TIM2_IRQ_NUMBER);
        TIM2_CR1.write_volatile(1); // CEN
    }
}

#[inline]
fn enable_fpu() {
    // CP10/CP11 full access, equivalent to the C startup's FPU enable.
    const CPACR: *mut u32 = 0xE000_ED88 as *mut u32;
    unsafe {
        CPACR.write_volatile(CPACR.read_volatile() | (0xFu32 << 20));
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
    }
}

fn enable_cycle_counter() {
    const DEMCR: *mut u32 = 0xE000_EDFC as *mut u32;
    const DWT_CTRL: *mut u32 = 0xE000_1000 as *mut u32;
    const DWT_CYCCNT: *mut u32 = 0xE000_1004 as *mut u32;
    unsafe {
        DEMCR.write_volatile(DEMCR.read_volatile() | (1 << 24));
        DWT_CYCCNT.write_volatile(0);
        DWT_CTRL.write_volatile(DWT_CTRL.read_volatile() | 1);
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
