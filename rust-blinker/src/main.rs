#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use cortex_m::peripheral::{scb::SystemHandler, syst::SystClkSource, NVIC, SCB};
use cortex_m_rt::{entry, exception};
use panic_halt as _;
use stm32f4xx_hal::gpio::{GpioExt, Speed};
use stm32f4::stm32f407::interrupt;
use stm32f4xx_hal::pac;
use synopsys_usb_otg::UsbBus;
use usb_device::device::{UsbDevice, UsbDeviceBuilder, UsbVidPid};
use xcore407i_blinker::camera::{clear_demo_canvas, show_usb_status, update_clock_from_rtc};
use tinyusb_sys as tusb;
use xcore407i_blinker::rtc;
use xcore407i_blinker::usb_uvc::{
    usb_bus, F407UsbHs, UvcUsbClass,
};

const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;

// One timer interrupt per USB HS microframe (125 us, 8 kHz).
const USB_POLL_TIMER_HZ: u32 = 8_000;
const TIM2_PRIORITY: u8 = 2;
const SYSTICK_PRIORITY: u8 = 1;

static TICKS: AtomicU32 = AtomicU32::new(0);

const CPU_SAMPLE_INTERVAL_MS: u32 = 1000;
const CORE_CLOCK_HZ: u64 = 168_000_000;
static LAST_CPU_SAMPLE: AtomicU32 = AtomicU32::new(0);
static USB_FS_STATUS: AtomicU32 = AtomicU32::new(0);

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

    let mut core = cortex_m::Peripherals::take().unwrap();
    core.SCB.enable_fpu();

    let mut device = pac::Peripherals::take().unwrap();

    let rtc_ready = rtc::initialize();
    update_clock_from_rtc();

    enable_cycle_counter(&mut core);

    // initialize_demo_canvas();
    clear_demo_canvas(0x10);
    show_usb_status(b"USB FS: INIT");

    configure_board_pins(
        &mut device.RCC,
        device.GPIOA,
        device.GPIOB,
        device.GPIOC,
        device.GPIOH,
        device.GPIOI,
    );

    // Enable the OTG_FS peripheral clock; OTG_HS remains owned by the UVC driver.
    unsafe {
        const RCC_AHB2ENR: *mut u32 = 0x4002_384c as *mut u32;
        RCC_AHB2ENR.write_volatile(RCC_AHB2ENR.read_volatile() | (1 << 7));
    }

    // Initialize TinyUSB host on root-hub port 0 (OTG_FS).
    show_usb_status(b"USB FS: HOST INIT");
    if unsafe { tusb::tuh_init(0) } {
        USB_FS_STATUS.store(1, Ordering::Release);
    } else {
        USB_FS_STATUS.store(2, Ordering::Release);
    }

    // Keep the 1 ms application timebase. It is independent of USB polling.
    core.SYST.set_clock_source(SystClkSource::Core);
    core.SYST.set_reload(168_000 - 1); // 1 ms at 168 MHz
    core.SYST.clear_current();

    configure_exception_priorities(&mut core.SCB);

    core.SYST.enable_interrupt();
    core.SYST.enable_counter();

    let bus = usb_bus().expect("USB bus initialization failed");

    // The large endpoint buffers remain in their existing DMA-visible SRAM
    // allocation. The UsbDevice itself remains in CCM as in the original code.
    static mut CAMERA: MaybeUninit<BoardUvcClass> = MaybeUninit::uninit();

    #[unsafe(link_section = ".ccmram.usb_device")]
    static mut USB_DEVICE: MaybeUninit<BoardUsbDevice> = MaybeUninit::uninit();

    // Construct in place and publish raw pointers. No long-lived &mut reference
    // is kept in main; the TIM2 ISR creates the only mutable references.
    let camera_ptr = unsafe {
        (*ptr::addr_of_mut!(CAMERA)).as_mut_ptr()
    };

    unsafe {
        camera_ptr.write(
            UvcUsbClass::new(bus).expect("UVC USB class initialization failed")
        );
    }

    let usb_ptr = unsafe {
        (*ptr::addr_of_mut!(USB_DEVICE)).as_mut_ptr()
    };

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
    configure_usb_poll_timer(
        &device.RCC,
        &device.TIM2,
        &mut core.NVIC,
    );

    let mut last_rtc_sample = 0;

    loop {
        // USB polling and camera-class access now belong exclusively to TIM2.
        let now = TICKS.load(Ordering::Relaxed);

        if rtc_ready && now.wrapping_sub(last_rtc_sample) >= 1000 {
            last_rtc_sample = now;
            rtc::refresh_display_time();
            update_clock_from_rtc();
        }

        unsafe { tusb::tuh_task_ext(0, false); }

        let status = USB_FS_STATUS.swap(0, Ordering::AcqRel);
        let message: &[u8] = match status {
            1 => b"USB FS: WAITING FOR DEVICE",
            2 => b"USB FS: HOST INIT FAILED",
            3 => b"USB FS: DEVICE ENUMERATED",
            4 => b"USB FS: KEYBOARD MOUNTED",
            5 => b"USB FS: KEYBOARD REMOVED",
            6 => b"USB FS: KEY REPORT RECEIVED",
            7 => b"USB FS: DEVICE DESCRIPTOR",
            8 => b"USB FS: CONFIG DESCRIPTOR",
            _ => b"",
        };
        if !message.is_empty() { show_usb_status(message); }

        // rotozoom.render_next();
    }
}


#[interrupt]
fn OTG_FS() {
    unsafe { tusb::hcd_int_handler(0, true); }
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_enum_descriptor_device_cb(_dev_addr: u8, _desc: *const tusb::tusb_desc_device_t) {
    USB_FS_STATUS.store(7, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_enum_descriptor_configuration_cb(_dev_addr: u8, _cfg_index: u8, _desc: *const tusb::tusb_desc_configuration_t) -> bool {
    USB_FS_STATUS.store(8, Ordering::Release);
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_mount_cb(_dev_addr: u8) { USB_FS_STATUS.store(3, Ordering::Release); }

#[unsafe(no_mangle)]
pub extern "C" fn tuh_umount_cb(_dev_addr: u8) { USB_FS_STATUS.store(5, Ordering::Release); }

#[unsafe(no_mangle)]
pub extern "C" fn tuh_hid_mount_cb(dev_addr: u8, instance: u8, _desc: *const u8, _len: u16) {
    USB_FS_STATUS.store(4, Ordering::Release);
    unsafe { tusb::tuh_hid_receive_report(dev_addr, instance); }
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_hid_umount_cb(_dev_addr: u8, _instance: u8) { USB_FS_STATUS.store(5, Ordering::Release); }

#[unsafe(no_mangle)]
pub extern "C" fn tuh_hid_report_received_cb(dev_addr: u8, instance: u8, _report: *const u8, _len: u16) {
    USB_FS_STATUS.store(6, Ordering::Release);
    unsafe { tusb::tuh_hid_receive_report(dev_addr, instance); }
}

/// TIM2 fires at 8 kHz. This is the only place that calls UsbDevice::poll().
#[interrupt]
fn TIM2() {
    // Obtain the TIM2 register block for interrupt-flag handling.
    let tim2 = unsafe { &*pac::TIM2::ptr() };

    // Clear the update interrupt flag.
    tim2.sr().write(|w| w.uif().clear_bit());

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

        camera.set_cpu_metrics(
            100 - busy_percent,
            generate_percent,
            submit_percent,
        );
    }
    */
}

/// Set a simple preemption-priority grouping and the system-handler priorities.
/// STM32F407 implements four priority bits, stored in the upper nibble.
fn configure_exception_priorities(scb: &mut SCB) {
    const SCB_AIRCR: *mut u32 = 0xE000_ED0C as *mut u32;

    unsafe {
        // PRIGROUP=0: all implemented priority bits are preemption priority.
        let aircr = SCB_AIRCR.read_volatile();
        SCB_AIRCR.write_volatile((aircr & !(7 << 8)) | (0x5FA << 16));

        // Cortex-M API handles the system-handler priority register layout.
        scb.set_priority(SystemHandler::SysTick, SYSTICK_PRIORITY << 4);

        // This Synopsys driver is polled; it does not provide an OTG_HS NVIC
        // handler. Keep OTG_HS disabled so the core's GINT output cannot vector
        // into the default handler. UsbBus::poll reads and services GINTSTS.
        NVIC::mask(interrupt::OTG_HS);
        NVIC::unpend(interrupt::OTG_HS);
    }
}

/// Configure TIM2 for an exact 125 us update period and enable its NVIC line.
fn configure_usb_poll_timer(
    rcc: &pac::RCC,
    tim2: &pac::TIM2,
    nvic: &mut NVIC,
) {
    // TIM2CLK = 84 MHz, PSC = 0, ARR = 10499 => 8,000 updates/second.
    let timer_ticks = 84_000_000 / USB_POLL_TIMER_HZ;

    // Enable TIM2 on APB1.
    rcc.apb1enr().modify(|_, w| w.tim2en().set_bit());

    // Reset TIM2 to its known reset state.
    rcc.apb1rstr().modify(|_, w| w.tim2rst().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim2rst().clear_bit());

    // Stop timer and disable update interrupt while configuring.
    tim2.cr1().write(|w| w.cen().clear_bit());
    tim2.dier().write(|w| w.uie().clear_bit());

    // PSC and CNT are zero after the peripheral reset.
    // ARR = 10499 gives a 125 us update period at 84 MHz.
    tim2.arr().write(|w| unsafe {
        w.arr().bits(timer_ticks - 1)
    });

    // Generate an update event to load the prescaler and auto-reload.
    tim2.egr().write(|w| w.ug().set_bit());

    // Clear the update flag generated by UG.
    tim2.sr().write(|w| w.uif().clear_bit());

    // Configure NVIC priority and clear any pending interrupt.
    unsafe {
        nvic.set_priority(interrupt::TIM2, TIM2_PRIORITY << 4);
        NVIC::unmask(interrupt::TIM2);
    }

    NVIC::unpend(interrupt::TIM2);

    // Enable update interrupt and start the timer.
    tim2.dier().write(|w| w.uie().set_bit());
    tim2.cr1().write(|w| w.cen().set_bit());
}

fn enable_cycle_counter(core: &mut cortex_m::Peripherals) {
    // DWT access is provided by cortex-m; global trace must be enabled first.
    core.DCB.enable_trace();
    core.DWT.set_cycle_count(0);
    core.DWT.enable_cycle_counter();
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
        RCC_APB1ENR.write_volatile(RCC_APB1ENR.read_volatile() | (1 << 28));
        PWR_CR.write_volatile(
            (PWR_CR.read_volatile() & !(3 << 14)) | (3 << 14)
        ); // scale 1

        FLASH_ACR.write_volatile((1 << 10) | (1 << 9) | (1 << 8) | 5);
        // caches + 5 WS

        RCC_CR.write_volatile(RCC_CR.read_volatile() | (1 << 16)); // HSEON
        while RCC_CR.read_volatile() & (1 << 17) == 0 {}

        // PLLM=4, PLLN=168, PLLP=/2, PLL source=HSE, PLLQ=7.
        RCC_PLLCFGR.write_volatile(4 | (168 << 6) | (1 << 22) | (7 << 24));

        RCC_CR.write_volatile(RCC_CR.read_volatile() | (1 << 24)); // PLLON
        while RCC_CR.read_volatile() & (1 << 25) == 0 {}

        // HCLK=/1, APB1=/4, APB2=/2, then switch SYSCLK to PLL.
        let cfgr = RCC_CFGR.read_volatile();
        let cfgr = (cfgr & !((0xf << 4) | (7 << 10) | (7 << 13) | 3))
            | (5 << 10)
            | (4 << 13);

        RCC_CFGR.write_volatile(cfgr | 2);

        while RCC_CFGR.read_volatile() & (3 << 2) != (2 << 2) {}
    }
}

/// Configure the Ethernet MDIO/MDC and USB3300 ULPI pin muxes.
///
/// The HAL owns the GPIO register layout and alternate-function mapping. The
/// pins are intentionally dropped after configuration; their hardware state
/// remains configured for the peripheral.
fn configure_board_pins(
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

    // USB FS host data pins use the internal PHY (AF10).
    let _usb_fs_pins = (
        a.pa11.into_alternate::<10>().speed(Speed::VeryHigh), // OTG_FS_DM
        a.pa12.into_alternate::<10>().speed(Speed::VeryHigh), // OTG_FS_DP
    );

    // XCore407I schematic: PC1 is FS_VBUSEN / OTG_PWR_OUT. The FS port's
    // MIC2025/75 power switch is enabled by driving EN low. PC1 is also
    // routed as Ethernet MDC on this board, so Ethernet MDIO management is
    // intentionally disabled while USB FS host VBUS is enabled.
    unsafe {
        const GPIOC_BASE: usize = 0x4002_0800;
        let bsrr = (GPIOC_BASE + 0x18) as *mut u32;
        let moder = GPIOC_BASE as *mut u32;
        let otyper = (GPIOC_BASE + 0x04) as *mut u32;
        let ospeedr = (GPIOC_BASE + 0x08) as *mut u32;
        let pupdr = (GPIOC_BASE + 0x0c) as *mut u32;

        // Drive low before switching the pin from its reset input state.
        bsrr.write_volatile(1 << (1 + 16));
        moder.write_volatile((moder.read_volatile() & !(0b11 << 2)) | (0b01 << 2));
        otyper.write_volatile(otyper.read_volatile() & !(1 << 1));
        ospeedr.write_volatile(ospeedr.read_volatile() & !(0b11 << 2));
        pupdr.write_volatile(pupdr.read_volatile() & !(0b11 << 2));
    }

    // USB3300 ULPI: all data/control pins use AF10 except PA4 SOF (AF12).
    // The original firmware selected very-high GPIO speed and no pull resistors.
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

