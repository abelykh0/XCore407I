#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use cortex_m::peripheral::NVIC;
use cortex_m_rt::entry;
use panic_halt as _;
use stm32f4::stm32f407::{interrupt, Interrupt};
use stm32f4xx_hal::pac;
use synopsys_usb_otg::UsbBus;
use usb_device::descriptor::lang_id::LangID;
use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};

use xcore407i_usb_mtp::{
    board,
    bus::usb_bus,
    mtp::MtpUsbClass,
    nand_fs,
    usb_hs::F407UsbHs,
};

const USB_POLL_HZ: u32 = 8_000;
const TIM2_PRIORITY: u8 = 2;

type BoardUsbBus = UsbBus<F407UsbHs>;
type BoardUsbDevice = UsbDevice<'static, BoardUsbBus>;
type BoardMtpClass = MtpUsbClass<'static, BoardUsbBus>;

// Initialized and published before TIM2 is enabled. TIM2 is the sole owner
// of mutable USB-device/class access while polling.
static USB_DEVICE_PTR: AtomicPtr<BoardUsbDevice> = AtomicPtr::new(ptr::null_mut());
static MTP_PTR: AtomicPtr<BoardMtpClass> = AtomicPtr::new(ptr::null_mut());

#[unsafe(link_section = ".ccmram.usb_device")]
static mut USB_DEVICE: MaybeUninit<BoardUsbDevice> = MaybeUninit::uninit();
static mut MTP: MaybeUninit<BoardMtpClass> = MaybeUninit::uninit();

#[entry]
fn main() -> ! {
    board::configure_system_clock_168mhz();

    let peripherals = pac::Peripherals::take().unwrap();
    let mut core = cortex_m::Peripherals::take().unwrap();
    let mut rcc = peripherals.RCC;

    board::configure_ulpi(
        &mut rcc,
        peripherals.GPIOA,
        peripherals.GPIOB,
        peripherals.GPIOC,
        peripherals.GPIOH,
        peripherals.GPIOI,
    );
    board::configure_nand(&mut rcc, peripherals.GPIOD, peripherals.GPIOE);

    // Start USB polling before NAND initialization/formatting. TIM2 continues
    // servicing USB while the main thread accesses NAND.
    let bus = usb_bus().expect("USB HS bus already initialized");
    let mtp_ptr = unsafe { (*ptr::addr_of_mut!(MTP)).as_mut_ptr() };
    unsafe {
        mtp_ptr.write(MtpUsbClass::new(bus));
        (*mtp_ptr).set_readme(b"Initializing NAND LittleFS...\\r\\n");
    }

    let strings = [
        StringDescriptors::new(LangID::EN_US)
            .manufacturer("XCore407I")
            .product("XCore407I NAND")
            .serial_number("XCORE407I-002"),
    ];
    let usb_ptr = unsafe { (*ptr::addr_of_mut!(USB_DEVICE)).as_mut_ptr() };
    unsafe {
        usb_ptr.write(
            UsbDeviceBuilder::new(bus, UsbVidPid(0x1209, 0x4071))
                .strings(&strings)
                .unwrap()
                .max_packet_size_0(64)
                .unwrap()
                .build(),
        );
    }

    MTP_PTR.store(mtp_ptr, Ordering::Release);
    USB_DEVICE_PTR.store(usb_ptr, Ordering::Release);
    configure_usb_poll_timer(&rcc, &peripherals.TIM2, &mut core.NVIC);

    // TIM2 owns all USB polling; NAND initialization and LittleFS formatting
    // run in the main thread without stopping USB enumeration.
    let (report, report_len) = nand_fs::initialize_and_mount_report();

    // Update the MTP object between polling interrupts. Mask TIM2 to ensure
    // the ISR cannot access MTP while set_readme() changes its contents.
    unsafe { NVIC::mask(Interrupt::TIM2); }
    let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
    let _ = mtp.set_readme(&report[..report_len]);
    peripherals.TIM2.sr().write(|w| w.uif().clear_bit());
    unsafe {
        NVIC::unpend(Interrupt::TIM2);
        NVIC::unmask(Interrupt::TIM2);
    }

    // Keep LittleFS mounted for the firmware lifetime. The closure owns the
    // mount, so later main-loop MTP requests can be serviced without remounting
    // or exposing NAND operations to the TIM2 USB polling interrupt.
    let mounted = nand_fs::with_mounted::<()>(|_fs| loop {
        core::hint::spin_loop();
    });
    let _ = mounted;
    loop {
        core::hint::spin_loop();
    }
}

/// Poll the USB HS device at 8 kHz (one call per 125 us microframe).
#[interrupt]
fn TIM2() {
    let tim2 = unsafe { &*pac::TIM2::ptr() };
    tim2.sr().write(|w| w.uif().clear_bit());

    let usb_ptr = USB_DEVICE_PTR.load(Ordering::Acquire);
    let mtp_ptr = MTP_PTR.load(Ordering::Acquire);
    if usb_ptr.is_null() || mtp_ptr.is_null() {
        return;
    }

    // SAFETY: the pointers are initialized before TIM2 is enabled. Main only
    // mutates MTP with TIM2 masked; all other USB access belongs to this ISR.
    let usb = unsafe { &mut *usb_ptr };
    let mtp = unsafe { &mut *mtp_ptr };
    let _ = usb.poll(&mut [mtp]);
}

fn configure_usb_poll_timer(rcc: &pac::RCC, tim2: &pac::TIM2, nvic: &mut NVIC) {
    // APB1 timer clock is 84 MHz with SYSCLK=168 MHz and APB1 prescaler /4.
    // PSC=0, ARR=10499 => 8 kHz / 125 us.
    let timer_ticks = 84_000_000 / USB_POLL_HZ;

    rcc.apb1enr().modify(|_, w| w.tim2en().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim2rst().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim2rst().clear_bit());

    tim2.cr1().write(|w| w.cen().clear_bit());
    tim2.dier().write(|w| w.uie().clear_bit());
    tim2.psc().write(|w| unsafe { w.psc().bits(0) });
    tim2.arr().write(|w| unsafe { w.arr().bits(timer_ticks - 1) });
    tim2.egr().write(|w| w.ug().set_bit());
    tim2.sr().write(|w| w.uif().clear_bit());

    unsafe {
        nvic.set_priority(Interrupt::TIM2, TIM2_PRIORITY << 4);
        NVIC::unpend(interrupt::TIM2);
    }

    tim2.dier().write(|w| w.uie().set_bit());
    tim2.cr1().write(|w| w.cen().set_bit());
    unsafe { NVIC::unmask(interrupt::TIM2); }
}
