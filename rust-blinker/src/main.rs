#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, Ordering};
use cortex_m::peripheral::{scb::SystemHandler, syst::SystClkSource, NVIC, SCB};
use cortex_m_rt::{entry, exception};
use panic_halt as _;
use stm32f4xx_hal::gpio::{GpioExt, Speed};
use stm32f4::stm32f407::interrupt;
use stm32f4xx_hal::pac;
use synopsys_usb_otg::UsbBus;
use usb_device::device::{UsbDevice, UsbDeviceBuilder, UsbVidPid};
use xcore407i_blinker::camera::{clear_demo_canvas, request_camera_mode_toggle, service_camera_mode_change, show_usb_status};
use tinyusb_sys as tusb;
use xcore407i_blinker::rtc;
//use xcore407i_blinker::nand_device;
use xcore407i_blinker::usb_uvc::{
    usb_bus, F407UsbHs, UvcUsbClass,
};

const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;

// One timer interrupt per USB HS microframe (125 us, 8 kHz).
const USB_POLL_TIMER_HZ: u32 = 8_000;
const USB_HOST_TASK_HZ: u32 = 1_000;
const TIM2_PRIORITY: u8 = 2;
const TIM3_PRIORITY: u8 = 3;
const SYSTICK_PRIORITY: u8 = 1;

static TICKS: AtomicU32 = AtomicU32::new(0);

// TinyUSB's no-RTOS time source. SysTick increments TICKS once per millisecond.
#[unsafe(no_mangle)]
pub extern "C" fn tusb_time_millis_api() -> u32 {
    TICKS.load(Ordering::Relaxed)
}

// DWC2 STM32 clock setup reads this CMSIS system-clock symbol.
// system_clock_168mhz() configures SYSCLK to 168 MHz before USB initialization.
#[unsafe(no_mangle)]
pub static SystemCoreClock: u32 = 168_000_000;


const CPU_SAMPLE_INTERVAL_MS: u32 = 1000;
const CORE_CLOCK_HZ: u64 = 168_000_000;
static LAST_CPU_SAMPLE: AtomicU32 = AtomicU32::new(0);
static USB_FS_STATUS: AtomicU32 = AtomicU32::new(0);
// TIM3 schedules TinyUSB host work; the actual task runs in thread context.
static USB_HOST_TASK_PENDING: AtomicBool = AtomicBool::new(false);

// HID boot-keyboard state. TinyUSB invokes HID callbacks from tuh_task_ext(),
// so these non-atomic key snapshots are only accessed serially there.
static mut PREVIOUS_KEYS: [u8; 6] = [0; 6];
static KEYBOARD_LOCKS: AtomicU8 = AtomicU8::new(0);
static KEYBOARD_DEVICE: AtomicU8 = AtomicU8::new(0);
static KEYBOARD_INSTANCE: AtomicU8 = AtomicU8::new(0);
static mut KEYBOARD_LED_REPORT: u8 = 0;
// Characters decoded by TinyUSB are handed to the emulator from the main loop.
static mut KEYBOARD_INPUT_BUFFER: [u8; 32] = [0; 32];
static KEYBOARD_INPUT_LEN: AtomicU8 = AtomicU8::new(0);

unsafe extern "C" {
    fn bk_platform_init(monitor_rom: *const u8, basic_rom: *const u8, basic_size_bytes: u32);
    fn bk_loop() -> i32;
    fn sendInput(buffer: *mut u8, size: u8);
}

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
    // update_clock_from_rtc(); // Temporarily disable the RTC clock overlay.

    enable_cycle_counter(&mut core);

    // BK emulator initialization
    unsafe {
         bk_platform_init(
             xcore407i_blinker::rom::MONITOR_ROM.as_ptr(),
             xcore407i_blinker::rom::BASIC_ROM.as_ptr(),
             xcore407i_blinker::rom::BASIC_ROM.len() as u32,
         );
    }

    // initialize_demo_canvas();
    clear_demo_canvas(0x10);
    show_usb_status(b"USB FS: INIT");

    configure_board_pins(
        &mut device.RCC,
        device.GPIOA,
        device.GPIOB,
        device.GPIOC,
        device.GPIOD,
        device.GPIOE,
        device.GPIOH,
        device.GPIOI,
    );

    // Non-destructive NAND identification and factory bad-block scan.
    // No blocks are erased or programmed.
    //nand_device::run_read_only_diagnostic();

    // Start the 1 ms timebase before TinyUSB initialization. TinyUSB's
    // no-RTOS port uses this clock during controller initialization.
    core.SYST.set_clock_source(SystClkSource::Core);
    core.SYST.set_reload(168_000 - 1); // 1 ms at 168 MHz
    core.SYST.clear_current();
    configure_exception_priorities(&mut core.SCB);
    core.SYST.enable_interrupt();
    core.SYST.enable_counter();

    // Enable and reset OTG_FS before handing it to TinyUSB. OTG_HS remains
    // owned by the UVC driver and is not reset here.
    unsafe {
        const RCC_AHB2ENR: *mut u32 = 0x4002_3834 as *mut u32;
        const RCC_AHB2RSTR: *mut u32 = 0x4002_3814 as *mut u32;
        RCC_AHB2ENR.write_volatile(RCC_AHB2ENR.read_volatile() | (1 << 7));
        let _ = RCC_AHB2ENR.read_volatile(); // complete the clock-enable write

        RCC_AHB2RSTR.write_volatile(RCC_AHB2RSTR.read_volatile() | (1 << 7));
        for _ in 0..16 { core::arch::asm!("nop"); }
        RCC_AHB2RSTR.write_volatile(RCC_AHB2RSTR.read_volatile() & !(1 << 7));
        let _ = RCC_AHB2RSTR.read_volatile();
    }

    // Prepare the OTG_FS interrupt while keeping it masked until TinyUSB has
    // initialized the host controller and installed its interrupt sources.
    unsafe {
        core.NVIC.set_priority(interrupt::OTG_FS, 0x3 << 4);
        NVIC::unpend(interrupt::OTG_FS);
        NVIC::mask(interrupt::OTG_FS);
    }

    // Initialize TinyUSB host on root-hub port 0 (OTG_FS).
    show_usb_status(b"USB FS: HOST INIT");
    let rh_init = tusb::tusb_rhport_init_t {
        role: tusb::tusb_role_t::TUSB_ROLE_HOST,
        speed: tusb::tusb_speed_t::TUSB_SPEED_FULL,
    };
    if unsafe { tusb::tuh_rhport_init(0, &rh_init) } {
        USB_FS_STATUS.store(1, Ordering::Release);
        unsafe { NVIC::unmask(interrupt::OTG_FS); }
    } else {
        USB_FS_STATUS.store(2, Ordering::Release);
    }

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

    // Start the independent USB service timers only after all USB objects
    // are initialized and published. TIM2 services the HS UVC device at 8 kHz.
    configure_usb_poll_timer(
        &device.RCC,
        &device.TIM2,
        &mut core.NVIC,
    );

    // TIM3 schedules the TinyUSB FS host task at 1 kHz. The ISR only sets a
    // flag; tuh_task_ext() itself must run in thread context, never in an ISR.
    configure_usb_host_task_timer(
        &device.RCC,
        &device.TIM3,
        &mut core.NVIC,
    );

    // let mut last_rtc_sample = 0; // RTC clock overlay temporarily disabled.

    loop {
        // BK instruction execution and keyboard forwarding
        unsafe { bk_loop(); }
        let input_len = KEYBOARD_INPUT_LEN.swap(0, Ordering::AcqRel);
        if input_len > 0 {
            let mut buffer = [0u8; 32];
            unsafe {
                ptr::copy_nonoverlapping(
                    core::ptr::addr_of!(KEYBOARD_INPUT_BUFFER).cast::<u8>(),
                    buffer.as_mut_ptr(),
                    input_len as usize,
                );
                sendInput(buffer.as_mut_ptr(), input_len);
            }
        }

        // Canvas clearing is deferred to thread context; TIM2 emits black
        // frames until the new mode's canvas has been cleared.
        service_camera_mode_change();

        // USB polling and camera-class access now belong exclusively to TIM2.
        let now = TICKS.load(Ordering::Relaxed);

        // if rtc_ready && now.wrapping_sub(last_rtc_sample) >= 1000 {
        //     last_rtc_sample = now;
        //     rtc::refresh_display_time();
        //     update_clock_from_rtc();
        // } // Temporarily disable the RTC clock overlay.

        if USB_HOST_TASK_PENDING.swap(false, Ordering::AcqRel) {
            unsafe { tusb::tuh_task_ext(0, false); }
        }
/*
        let status = USB_FS_STATUS.swap(0, Ordering::AcqRel);
        let message: &[u8] = match status {
            1 => b"USB FS: WAITING FOR DEVICE",
            2 => b"USB FS: HOST INIT FAILED",
            3 => b"USB FS: DEVICE ENUMERATED",
            4 => b"USB FS: KEYBOARD MOUNTED",
            5 => b"USB FS: KEYBOARD REMOVED",
            6 => b"USB FS: KEYBOARD INPUT",
            7 => b"USB FS: DEVICE DESCRIPTOR",
            8 => b"USB FS: CONFIG DESCRIPTOR",
            9 => b"USB FS: BOOT PROTOCOL FAILED",
            _ => b"",
        };
        if !message.is_empty() { show_usb_status(message); }
*/
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
    // HID interface protocol 1 is a boot keyboard; ignore unrelated HID
    // interfaces rather than decoding a mouse/gamepad report as keystrokes.
    if unsafe { tusb::tuh_hid_interface_protocol(dev_addr, instance) } != 1 {
        return;
    }
    KEYBOARD_DEVICE.store(dev_addr, Ordering::Release);
    KEYBOARD_INSTANCE.store(instance, Ordering::Release);
    KEYBOARD_LOCKS.store(0, Ordering::Release);
    unsafe {
        PREVIOUS_KEYS = [0; 6];
        KEYBOARD_LED_REPORT = 0;
    }
    USB_FS_STATUS.store(4, Ordering::Release);

    // TinyUSB normally selects boot protocol for boot keyboards during
    // enumeration (CFG_TUH_HID_SET_PROTOCOL_ON_ENUM). Verify that before
    // interpreting reports as the standard 8-byte boot-keyboard packet.
    if unsafe { tusb::tuh_hid_get_protocol(dev_addr, instance) } != 0 {
        // Request boot protocol; the completion callback starts reception.
        if !unsafe { tusb::tuh_hid_set_protocol(dev_addr, instance, 0) } {
            USB_FS_STATUS.store(9, Ordering::Release);
        }
    } else {
        unsafe { tusb::tuh_hid_receive_report(dev_addr, instance); }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_hid_umount_cb(dev_addr: u8, instance: u8) {
    if KEYBOARD_DEVICE.load(Ordering::Acquire) == dev_addr
        && KEYBOARD_INSTANCE.load(Ordering::Acquire) == instance
    {
        KEYBOARD_DEVICE.store(0, Ordering::Release);
        KEYBOARD_INSTANCE.store(0, Ordering::Release);
        KEYBOARD_LOCKS.store(0, Ordering::Release);
        unsafe { PREVIOUS_KEYS = [0; 6]; }
    }
    USB_FS_STATUS.store(5, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_hid_set_protocol_complete_cb(_dev_addr: u8, _instance: u8, protocol: u8) {
    if protocol == 0 {
        unsafe { tusb::tuh_hid_receive_report(
            KEYBOARD_DEVICE.load(Ordering::Acquire),
            KEYBOARD_INSTANCE.load(Ordering::Acquire),
        ); }
    } else {
        USB_FS_STATUS.store(9, Ordering::Release);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tuh_hid_report_received_cb(
    dev_addr: u8,
    instance: u8,
    report: *const u8,
    len: u16,
) {
    if len >= 8 && !report.is_null() {
        let bytes = unsafe { core::slice::from_raw_parts(report, len as usize) };
        process_boot_keyboard_report(dev_addr, instance, bytes);
    }
    // Re-arm the interrupt-IN endpoint after every completed report.
    unsafe { tusb::tuh_hid_receive_report(dev_addr, instance); }
}

/// Process an 8-byte USB HID boot-keyboard report:
/// [modifier, reserved, key1..key6].
fn process_boot_keyboard_report(dev_addr: u8, instance: u8, report: &[u8]) {
    if KEYBOARD_DEVICE.load(Ordering::Acquire) != dev_addr
        || KEYBOARD_INSTANCE.load(Ordering::Acquire) != instance
    {
        return;
    }

    let modifiers = report[0];
    let shift = modifiers & 0x22 != 0;
    let control_or_alt = modifiers & 0xdd != 0;
    let locks_before = KEYBOARD_LOCKS.load(Ordering::Acquire);
    let mut locks = locks_before;
    let mut text = [b' '; 32];
    let mut used = 0usize;
    let mut emulator_input = [0u8; 32];
    let mut emulator_used = 0usize;

    let previous = unsafe { PREVIOUS_KEYS };
    let mut current = [0u8; 6];
    current.copy_from_slice(&report[2..8]);

    for &key in current.iter() {
        if key == 0 || previous.contains(&key) {
            continue;
        }

        // F1 requests a camera mode change. The camera applies it at the
        // next UVC frame boundary and clears the canvas in the new format.
        if key == 0x3a {
            request_camera_mode_toggle();
            continue;
        }

        // HID keyboard usages for the three toggle keys.
        match key {
            0x39 => locks ^= 0x02, // Caps Lock
            0x53 => locks ^= 0x01, // Num Lock
            0x47 => locks ^= 0x04, // Scroll Lock
            _ => {}
        }

        if let Some(ch) = key_to_ascii(key, shift, locks & 0x02 != 0, locks & 0x01 != 0, control_or_alt) {
            append_keyboard_char(&mut text, &mut used, ch);
            append_keyboard_char(&mut emulator_input, &mut emulator_used, ch);
        } else if let Some(label) = special_key_label(key, locks & 0x01 != 0) {
            if used != 0 { append_keyboard_char(&mut text, &mut used, b' '); }
            append_keyboard_text(&mut text, &mut used, label);
        }
    }

    unsafe { PREVIOUS_KEYS = current; }

    if locks != locks_before {
        KEYBOARD_LOCKS.store(locks, Ordering::Release);
        // HID LED output bits: Num=bit0, Caps=bit1, Scroll=bit2.
        // SET_REPORT is asynchronous; the one-byte buffer is static and
        // remains valid until TinyUSB completes the control transfer.
        unsafe {
            KEYBOARD_LED_REPORT = locks & 0x07;
            let led_report = core::ptr::addr_of_mut!(KEYBOARD_LED_REPORT);
            let _ = tusb::tuh_hid_set_report(
                dev_addr, instance, 0, 2, led_report.cast(), 1,
            );
        }
        let event: &[u8] = match locks & 0x07 {
            0 => b"USB KEYBOARD: LOCKS OFF",
            1 => b"USB KEYBOARD: NUM LOCK",
            2 => b"USB KEYBOARD: CAPS LOCK",
            3 => b"USB KEYBOARD: CAPS + NUM LOCK",
            4 => b"USB KEYBOARD: SCROLL LOCK",
            5 => b"USB KEYBOARD: NUM + SCROLL LOCK",
            6 => b"USB KEYBOARD: CAPS + SCROLL LOCK",
            _ => b"USB KEYBOARD: ALL LOCKS",
        };
        show_usb_status(event);
    }

    // if used != 0 {
    //     show_keyboard_input(&text[..used]);
    // } // Keyboard diagnostic text temporarily disabled.

    // Only decoded key bytes go to the emulator; display labels are not BK input.
    if emulator_used != 0 {
        unsafe {
            ptr::copy_nonoverlapping(
                emulator_input.as_ptr(),
                core::ptr::addr_of_mut!(KEYBOARD_INPUT_BUFFER).cast::<u8>(),
                emulator_used,
            );
        }
        KEYBOARD_INPUT_LEN.store(emulator_used as u8, Ordering::Release);
    }
}

fn append_keyboard_char(out: &mut [u8; 32], used: &mut usize, ch: u8) {
    if *used < out.len() {
        out[*used] = ch;
        *used += 1;
    }
}

fn append_keyboard_text(out: &mut [u8; 32], used: &mut usize, text: &[u8]) {
    for &ch in text {
        append_keyboard_char(out, used, ch);
    }
}

fn key_to_ascii(key: u8, shift: bool, caps: bool, num: bool, control_or_alt: bool) -> Option<u8> {
    if control_or_alt { return None; }

    // Original BK OnKey() control codes for the cursor keys.
    // The keypad navigation usages act as arrows when Num Lock is off.
    match key {
        0x4f => return Some(0x19), // Right
        0x50 => return Some(0x08), // Left
        0x51 => return Some(0x1b), // Down
        0x52 => return Some(0x1a), // Up
        0x5a if !num => return Some(0x1b), // Keypad Down
        0x5c if !num => return Some(0x08), // Keypad Left
        0x5e if !num => return Some(0x19), // Keypad Right
        0x60 if !num => return Some(0x1a), // Keypad Up
        _ => {}
    }

    match key {
        0x04..=0x1d => {
            let lower = b'a' + (key - 0x04);
            Some(if shift ^ caps { lower - 32 } else { lower })
        }
        0x1e..=0x27 => {
            const NORMAL: &[u8; 10] = b"1234567890";
            const SHIFTED: &[u8; 10] = b"!@#$%^&*()";
            let i = (key - 0x1e) as usize;
            Some(if shift { SHIFTED[i] } else { NORMAL[i] })
        }
        0x28 => Some(b'\n'), // BK OnKey maps Enter to 0x0A.
        0x2a => Some(0x18), // BK OnKey maps Backspace to 0x18.
        0x2c => Some(b' '),
        0x2d => Some(if shift { b'_' } else { b'-' }),
        0x2e => Some(if shift { b'+' } else { b'=' }),
        0x2f => Some(if shift { b'{' } else { b'[' }),
        0x30 => Some(if shift { b'}' } else { b']' }),
        0x31 => Some(if shift { b'|' } else { b'\\' }),
        0x33 => Some(if shift { b':' } else { b';' }),
        0x34 => Some(if shift { b'"' } else { b'\'' }),
        0x35 => Some(if shift { b'~' } else { b'`' }),
        0x36 => Some(if shift { b'<' } else { b',' }),
        0x37 => Some(if shift { b'>' } else { b'.' }),
        0x38 => Some(if shift { b'?' } else { b'/' }),
        0x54 => Some(b'/'),
        0x55 => Some(b'*'),
        0x56 => Some(b'-'),
        0x57 => Some(b'+'),
        0x58 => Some(b'\n'),
        0x59..=0x61 if num => Some(b'1' + (key - 0x59)),
        0x62 if num => Some(b'0'),
        0x63 if num => Some(b'.'),
        _ => None,
    }
}

fn special_key_label(key: u8, num: bool) -> Option<&'static [u8]> {
    match key {
        0x28 => Some(b"[ENTER]"),
        0x29 => Some(b"[ESC]"),
        0x2a => Some(b"[BACKSPACE]"),
        0x2b => Some(b"[TAB]"),
        0x39 => Some(b"[CAPS]"),
        0x46 => Some(b"[PRTSC]"),
        0x47 => Some(b"[SCROLL]"),
        0x48 => Some(b"[PAUSE]"),
        0x49 => Some(b"[INS]"),
        0x4a => Some(b"[HOME]"),
        0x4b => Some(b"[PGUP]"),
        0x4c => Some(b"[DEL]"),
        0x4d => Some(b"[END]"),
        0x4e => Some(b"[PGDN]"),
        0x4f => Some(b"[RIGHT]"),
        0x50 => Some(b"[LEFT]"),
        0x51 => Some(b"[DOWN]"),
        0x52 => Some(b"[UP]"),
        0x53 => Some(b"[NUM]"),
        0x59 if !num => Some(b"[KP-END]"),
        0x5a if !num => Some(b"[KP-DOWN]"),
        0x5b if !num => Some(b"[KP-PGDN]"),
        0x5c if !num => Some(b"[KP-LEFT]"),
        0x5d if !num => Some(b"[KP5]"),
        0x5e if !num => Some(b"[KP-RIGHT]"),
        0x5f if !num => Some(b"[KP-HOME]"),
        0x60 if !num => Some(b"[KP-UP]"),
        0x61 if !num => Some(b"[KP-PGUP]"),
        0x62 if !num => Some(b"[KP-INS]"),
        0x63 if !num => Some(b"[KP-DEL]"),
        0x3a => Some(b"[F1]"),
        0x3b => Some(b"[F2]"),
        0x3c => Some(b"[F3]"),
        0x3d => Some(b"[F4]"),
        0x3e => Some(b"[F5]"),
        0x3f => Some(b"[F6]"),
        0x40 => Some(b"[F7]"),
        0x41 => Some(b"[F8]"),
        0x42 => Some(b"[F9]"),
        0x43 => Some(b"[F10]"),
        0x44 => Some(b"[F11]"),
        0x45 => Some(b"[F12]"),
        0x65 => Some(b"[MENU]"),
        _ => None,
    }
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

/// Configure TIM3 as a 1 kHz scheduler tick for the TinyUSB FS host task.
///
/// TinyUSB's host task dispatches callbacks and is not ISR-safe. TIM3 therefore
/// only requests a task run; main() performs the work in thread context.
fn configure_usb_host_task_timer(
    rcc: &pac::RCC,
    tim3: &pac::TIM3,
    nvic: &mut NVIC,
) {
    // TIM3CLK = 84 MHz (SYSCLK=168 MHz, APB1=/4, timer clock doubled).
    // PSC=83 divides 84 MHz to 1 MHz; ARR=999 gives a 1 kHz update rate.

    rcc.apb1enr().modify(|_, w| w.tim3en().set_bit());

    rcc.apb1rstr().modify(|_, w| w.tim3rst().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim3rst().clear_bit());

    tim3.cr1().write(|w| w.cen().clear_bit());
    tim3.dier().write(|w| w.uie().clear_bit());
    tim3.psc().write(|w| unsafe { w.psc().bits(83) });
    tim3.arr().write(|w| unsafe { w.arr().bits(999) });
    tim3.egr().write(|w| w.ug().set_bit());
    tim3.sr().write(|w| w.uif().clear_bit());

    unsafe {
        nvic.set_priority(interrupt::TIM3, TIM3_PRIORITY << 4);
        NVIC::unpend(interrupt::TIM3);
        NVIC::unmask(interrupt::TIM3);
    }

    tim3.dier().write(|w| w.uie().set_bit());
    tim3.cr1().write(|w| w.cen().set_bit());
}

#[interrupt]
fn TIM3() {
    let tim3 = unsafe { &*pac::TIM3::ptr() };
    tim3.sr().write(|w| w.uif().clear_bit());
    USB_HOST_TASK_PENDING.store(true, Ordering::Release);
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
    gpiod: pac::GPIOD,
    gpioe: pac::GPIOE,
    gpioh: pac::GPIOH,
    gpioi: pac::GPIOI,
) {
    let a = gpioa.split(rcc);
    let b = gpiob.split(rcc);
    let c = gpioc.split(rcc);
    let d = gpiod.split(rcc);
    let e = gpioe.split(rcc);
    let h = gpioh.split(rcc);
    let i = gpioi.split(rcc);

    // XCore407I NAND flash uses FSMC AF12. CLE=A17 (PD12), ALE=A16 (PD11),
    // NCE2=PD7, NWAIT=PD6, NWE=PD5, NOE=PD4, D0..D3=PD14/15/0/1,
    // and D4..D7=PE7..PE10.
    let _nand_pins = (
        d.pd0.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd1.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd4.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd5.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd6.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd7.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd11.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd12.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd14.into_alternate::<12>().speed(Speed::VeryHigh),
        d.pd15.into_alternate::<12>().speed(Speed::VeryHigh),
        e.pe7.into_alternate::<12>().speed(Speed::VeryHigh),
        e.pe8.into_alternate::<12>().speed(Speed::VeryHigh),
        e.pe9.into_alternate::<12>().speed(Speed::VeryHigh),
        e.pe10.into_alternate::<12>().speed(Speed::VeryHigh),
    );

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

