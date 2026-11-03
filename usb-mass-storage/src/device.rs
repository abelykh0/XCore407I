//! Complete USB HS MSC device wrapper for the XCore407I board.

use crate::block::{BlockDevice, BlockError};
use crate::msc::MassStorage;
use crate::usb_hs::F407UsbHs;
use synopsys_usb_otg::UsbBus;
use usb_device::bus::UsbBusAllocator;
use usb_device::device::{UsbDevice, UsbDeviceBuilder, UsbVidPid};

pub type BoardUsbBus = UsbBus<F407UsbHs>;

pub struct F407MassStorage<'a, D: BlockDevice> {
    device: UsbDevice<'a, BoardUsbBus>,
    class: MassStorage<'a, BoardUsbBus, D>,
}

impl<'a, D: BlockDevice> F407MassStorage<'a, D> {
    /// Allocate BOT bulk endpoints and construct the USB device descriptor.
    /// The caller must configure the 168 MHz clock and USB3300 ULPI pins first.
    pub fn new(bus: &'a UsbBusAllocator<BoardUsbBus>, storage: D) -> Self {
        let class = MassStorage::new(bus, storage);
        let device = UsbDeviceBuilder::new(bus, UsbVidPid(0x1209, 0x4072))
            .manufacturer("XCore407I")
            .product("NAND Mass Storage")
            .serial_number("XCORE407I-MSC")
            .max_packet_size_0(64)
            .unwrap()
            .build();
        Self { device, class }
    }

    /// Poll the USB device. Call continuously from thread context; do not call
    /// from the OTG_HS interrupt handler.
    pub fn poll(&mut self) -> bool {
        self.device.poll(&mut [&mut self.class])
    }

    /// Flush the disk-image cache before handing the LittleFS volume back to
    /// firmware-side code or before intentionally disconnecting USB.
    pub fn sync_storage(&mut self) -> Result<(), BlockError> {
        self.class.sync_storage()
    }

    pub fn storage_mut(&mut self) -> &mut D {
        self.class.storage_mut()
    }
}
