#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;
use stm32f4xx_hal::pac;
use usb_device::device::{UsbDeviceBuilder, UsbVidPid};

use xcore407i_usb_mtp::{
    board,
    bus::usb_bus,
    mtp::MtpUsbClass,
};

/// Minimal USB bring-up firmware.
///
/// This intentionally exposes only the MTP interface descriptors. No storage
/// is mounted and no MTP operations are serviced yet; this stage verifies
/// OTG_HS + ULPI initialization and host enumeration without touching NAND.
#[entry]
fn main() -> ! {
    board::configure_system_clock_168mhz();

    let peripherals = pac::Peripherals::take().unwrap();
    let mut rcc = peripherals.RCC;

    board::configure_ulpi(
        &mut rcc,
        peripherals.GPIOA,
        peripherals.GPIOB,
        peripherals.GPIOC,
        peripherals.GPIOH,
        peripherals.GPIOI,
    );

    let bus = usb_bus().expect("USB HS bus already initialized");
    let mut mtp = MtpUsbClass::new(bus);

    let mut usb_dev = UsbDeviceBuilder::new(bus, UsbVidPid(0x1209, 0x4071))
        .manufacturer("XCore407I")
        .product("MTP bring-up")
        .serial_number("XCORE407I-001")
        .max_packet_size_0(64)
        .unwrap()
        .build();

    loop {
        // Enumeration and control requests are handled by usb-device. The
        // class currently advertises endpoints only; it does not claim to
        // implement the MTP transaction protocol yet.
        let _ = usb_dev.poll(&mut [&mut mtp]);
        core::hint::spin_loop();
    }
}
