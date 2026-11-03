//! STM32F407 OTG_HS controller using the external ULPI PHY (USB3300).
//!
//! Board ULPI GPIO muxing and the 60 MHz ULPI clock must be configured before
//! constructing the UsbBus. The high-speed capability is explicit here;
//! this must not be replaced with OTG_FS or an internal full-speed PHY.

use synopsys_usb_otg::{PhyType, UsbPeripheral};

pub struct F407UsbHs;

unsafe impl UsbPeripheral for F407UsbHs {
    const REGISTERS: *const () = 0x4004_0000 as *const ();
    const HIGH_SPEED: bool = true;
    const FIFO_DEPTH_WORDS: usize = 1024;
    const ENDPOINT_COUNT: usize = 6;

    fn enable() {
        // OTG_HS and ULPI clock gates (AHB1ENR OTGHSEN / OTGHSULPIEN).
        let rcc_ahb1enr = 0x4002_3830 as *mut u32;
        unsafe {
            rcc_ahb1enr.write_volatile(rcc_ahb1enr.read_volatile() | (1 << 29) | (1 << 30));
            let _ = rcc_ahb1enr.read_volatile();
        }
    }

    fn ahb_frequency_hz(&self) -> u32 { 168_000_000 }
    fn phy_type(&self) -> PhyType { PhyType::ExternalHighSpeed }
}
