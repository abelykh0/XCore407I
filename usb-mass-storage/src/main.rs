#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;
use stm32f4xx_hal::pac;
use usb_device::descriptor::lang_id::LangID;
use usb_device::device::{StringDescriptors, UsbDeviceBuilder, UsbVidPid};

use xcore407i_usb_mtp::{
    board,
    bus::usb_bus,
    mtp::MtpUsbClass,
    nand_diag,
};

/// Minimal USB/MTP bring-up firmware.
///
/// Exposes a read-only embedded demo object over MTP. NAND and LittleFS are
/// deliberately not mounted in this stage.
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
    board::configure_nand(&mut rcc, peripherals.GPIOD, peripherals.GPIOE);

    // Scan factory bad-block markers before USB enumeration and publish the
    // result as README.txt over MTP. This diagnostic never erases/programs NAND.
    let (nand_report, nand_report_len) = nand_diag::create_report();

    let bus = usb_bus().expect("USB HS bus already initialized");
    let mut mtp = MtpUsbClass::new(bus);
    let _ = mtp.set_readme(&nand_report[..nand_report_len]);

    let strings = [
        StringDescriptors::new(LangID::EN_US)
            .manufacturer("XCore407I")
            .product("MTP bring-up")
            .serial_number("XCORE407I-001"),
    ];
    let mut usb_dev = UsbDeviceBuilder::new(bus, UsbVidPid(0x1209, 0x4071))
        .strings(&strings)
        .unwrap()
        .max_packet_size_0(64)
        .unwrap()
        .build();

    loop {
        // USB enumeration and MTP transactions are handled by the class.
        let _ = usb_dev.poll(&mut [&mut mtp]);
        core::hint::spin_loop();
    }
}
