//! Singleton USB HS bus allocator and DMA-visible endpoint-zero buffers.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};
use synopsys_usb_otg::UsbBus;
use usb_device::bus::UsbBusAllocator;
use crate::usb_hs::F407UsbHs;

type BoardBus = UsbBus<F407UsbHs>;
static TAKEN: AtomicBool = AtomicBool::new(false);

pub fn usb_bus() -> Result<&'static UsbBusAllocator<BoardBus>, ()> {
    TAKEN.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).map_err(|_| ())?;
    // usb-device control endpoint buffers: 256-byte OUT and 64-byte IN.
    // EP0 plus MTP bulk OUT/IN and event IN staging buffers. Must reside in
    // DMA-accessible SRAM (0x2000_0000), not the STM32F4 CCM region.
    static mut EP_MEMORY: [u32; 512] = [0; 512];
    static mut BUS: MaybeUninit<UsbBusAllocator<BoardBus>> = MaybeUninit::uninit();
    let memory = unsafe { &mut *core::ptr::addr_of_mut!(EP_MEMORY) };
    let bus = UsbBus::new(F407UsbHs, memory);
    let allocator = unsafe {
        (*core::ptr::addr_of_mut!(BUS)).write(bus);
        (*core::ptr::addr_of!(BUS)).assume_init_ref()
    };
    Ok(allocator)
}
