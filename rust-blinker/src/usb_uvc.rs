//! `usb-device` class adapter for the board's external USB3300 ULPI PHY.

use crate::uvc::{
    ControlResponse, UvcClass, ENDPOINT_IN, FRAME_BYTES, FRAME_INTERVAL_100NS, MAX_PACKET_SIZE,
};
use synopsys_usb_otg::{PhyType, UsbPeripheral};
use usb_device::bus::{InterfaceNumber, StringIndex, UsbBus, UsbBusAllocator};
use usb_device::class::{ControlIn, ControlOut, UsbClass};
use usb_device::control::{Recipient, Request, RequestType};
use usb_device::descriptor::DescriptorWriter;
use usb_device::endpoint::{
    EndpointAddress, EndpointIn, EndpointType, IsochronousSynchronizationType, IsochronousUsageType,
};
use usb_device::Result;

const VIDEO_CLASS: u8 = 0x0e;
const VIDEO_CONTROL_SUBCLASS: u8 = 0x01;
const VIDEO_STREAMING_SUBCLASS: u8 = 0x02;

/// F407's OTG_HS controller and external USB3300 PHY.
pub struct F407UsbHs;

// This is a register-compatible STM32 OTG_HS controller. The caller configures
// the board's ULPI GPIO mux and system clock before constructing UsbDevice.
unsafe impl UsbPeripheral for F407UsbHs {
    const REGISTERS: *const () = 0x4004_0000 as *const ();
    const HIGH_SPEED: bool = true;
    const FIFO_DEPTH_WORDS: usize = 1024;
    const ENDPOINT_COUNT: usize = 6;

    fn enable() {
        let rcc_ahb1enr = 0x4002_3830 as *mut u32;
        unsafe {
            rcc_ahb1enr.write_volatile(rcc_ahb1enr.read_volatile() | (1 << 29) | (1 << 30));
            let _ = rcc_ahb1enr.read_volatile();
        }
    }

    fn ahb_frequency_hz(&self) -> u32 {
        168_000_000
    }

    fn phy_type(&self) -> PhyType {
        PhyType::ExternalHighSpeed
    }
}

/// Configure the exact ULPI pin map used by `Core/Src/usb_otg.c`.
///
/// # Safety
/// The caller must run this during early boot before the OTG_HS peripheral is
/// enabled and must ensure this pin assignment is not shared with another
/// active peripheral.
pub unsafe fn configure_ulpi_pins() {
    const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
    unsafe {
        // GPIOA/B/C/H/I clocks.
        RCC_AHB1ENR.write_volatile(
            RCC_AHB1ENR.read_volatile() | (1 << 0) | (1 << 1) | (1 << 2) | (1 << 7) | (1 << 8),
        );
    }
    // (GPIO base, pin number, alternate function)
    let pins = [
        (0x4002_0000, 3, 10),  // PA3 ULPI D0
        (0x4002_0000, 5, 10),  // PA5 ULPI CLK
        (0x4002_0000, 4, 12),  // PA4 SOF
        (0x4002_0400, 0, 10),  // PB0 ULPI D1
        (0x4002_0400, 1, 10),  // PB1 ULPI D2
        (0x4002_0400, 10, 10), // PB10 ULPI D3
        (0x4002_0400, 11, 10), // PB11 ULPI D4
        (0x4002_0400, 12, 10), // PB12 ULPI D5
        (0x4002_0400, 13, 10), // PB13 ULPI D6
        (0x4002_0400, 5, 10),  // PB5 ULPI D7
        (0x4002_0800, 0, 10),  // PC0 ULPI STP
        (0x4002_1c00, 4, 10),  // PH4 ULPI NXT
        (0x4002_2000, 11, 10), // PI11 ULPI DIR
    ];
    for (base, pin, af) in pins {
        unsafe { configure_alternate_pin(base, pin, af) };
    }
}

unsafe fn configure_alternate_pin(base: usize, pin: u32, af: u32) {
    let moder = (base as *mut u32).add(0);
    let otyper = (base as *mut u32).add(1);
    let ospeedr = (base as *mut u32).add(2);
    let pupdr = (base as *mut u32).add(3);
    let afr = (base as *mut u32).add(8 + (pin / 8) as usize);
    let shift = pin * 2;
    let af_shift = (pin % 8) * 4;
    unsafe {
        moder.write_volatile((moder.read_volatile() & !(3 << shift)) | (2 << shift));
        otyper.write_volatile(otyper.read_volatile() & !(1 << pin));
        ospeedr.write_volatile((ospeedr.read_volatile() & !(3 << shift)) | (3 << shift));
        pupdr.write_volatile(pupdr.read_volatile() & !(3 << shift));
        afr.write_volatile((afr.read_volatile() & !(0xf << af_shift)) | (af << af_shift));
    }
}

pub struct UvcUsbClass<'a, B: UsbBus> {
    function: UvcClass,
    control_interface: InterfaceNumber,
    streaming_interface: InterfaceNumber,
    interface_string: StringIndex,
    endpoint: EndpointIn<'a, B>,
}

impl<'a, B: UsbBus> UvcUsbClass<'a, B> {
    pub fn new(allocator: &'a UsbBusAllocator<B>) -> Self {
        let control_interface = allocator.interface();
        let streaming_interface = allocator.interface();
        let interface_string = allocator.string();
        let endpoint = allocator
            .alloc(
                Some(EndpointAddress::from(ENDPOINT_IN)),
                EndpointType::Isochronous {
                    synchronization: IsochronousSynchronizationType::Asynchronous,
                    usage: IsochronousUsageType::Data,
                },
                MAX_PACKET_SIZE,
                1,
            )
            .expect("failed to allocate UVC high-speed isochronous IN endpoint");
        Self {
            function: UvcClass::new(),
            control_interface,
            streaming_interface,
            interface_string,
            endpoint,
        }
    }

    fn write_video_control_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(
            self.control_interface,
            VIDEO_CLASS,
            VIDEO_CONTROL_SUBCLASS,
            0,
        )?;
        // UVC 1.1 VideoControl header (two-byte descriptor header is added by writer).
        writer.write(
            0x24,
            &[
                1,
                0x10,
                0x01,
                30,
                0,
                0,
                0x6c,
                0xdc,
                0x02,
                1,
                u8::from(self.streaming_interface),
            ],
        )?;
        // Vendor-specific input terminal and USB streaming output terminal.
        writer.write(0x24, &[2, 1, 0x00, 0x02, 0, 0])?;
        writer.write(0x24, &[3, 2, 0x01, 0x01, 0, 1, 0])?;
        Ok(())
    }

    fn write_video_streaming_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface_alt(
            self.streaming_interface,
            0,
            VIDEO_CLASS,
            VIDEO_STREAMING_SUBCLASS,
            0,
            Some(self.interface_string),
        )?;
        let endpoint = self.endpoint.address().into();
        // VS input header: all VS format descriptors together occupy 77 bytes.
        writer.write(0x24, &[1, 1, 77, 0, endpoint, 0, 2, 0, 0, 0, 1, 0])?;
        // NV12 GUID and 12 bits per pixel.
        writer.write(
            0x24,
            &[
                4, 1, 1, 0x4e, 0x56, 0x31, 0x32, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b,
                0x71, 12, 1, 0, 0, 0, 0,
            ],
        )?;
        let mut frame = [0u8; 28];
        frame[0..3].copy_from_slice(&[5, 1, 0]);
        frame[3..5].copy_from_slice(&512u16.to_le_bytes());
        frame[5..7].copy_from_slice(&512u16.to_le_bytes());
        let bitrate = 512u32 * 512 * 16 * 12;
        frame[7..11].copy_from_slice(&bitrate.to_le_bytes());
        frame[11..15].copy_from_slice(&bitrate.to_le_bytes());
        frame[15..19].copy_from_slice(&FRAME_BYTES.to_le_bytes());
        frame[19..23].copy_from_slice(&FRAME_INTERVAL_100NS.to_le_bytes());
        frame[23] = 1;
        frame[24..28].copy_from_slice(&FRAME_INTERVAL_100NS.to_le_bytes());
        writer.write(0x24, &frame)?;
        writer.write(0x24, &[0x0d, 1, 1, 4])?;

        writer.interface_alt(
            self.streaming_interface,
            1,
            VIDEO_CLASS,
            VIDEO_STREAMING_SUBCLASS,
            0,
            Some(self.interface_string),
        )?;
        writer.endpoint(&self.endpoint)?;
        Ok(())
    }

    fn accepts_control(&self, request: &Request) -> bool {
        request.request_type == RequestType::Class
            && request.recipient == Recipient::Interface
            && (request.index as u8 == u8::from(self.control_interface)
                || request.index as u8 == u8::from(self.streaming_interface))
    }
}

impl<B: UsbBus> UsbClass<B> for UvcUsbClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.iad(
            self.control_interface,
            2,
            VIDEO_CLASS,
            0x03,
            0,
            Some(self.interface_string),
        )?;
        self.write_video_control_descriptors(writer)?;
        self.write_video_streaming_descriptors(writer)
    }

    fn get_string(
        &self,
        index: StringIndex,
        _lang_id: usb_device::descriptor::lang_id::LangID,
    ) -> Option<&str> {
        if index == self.interface_string {
            Some("Static 512x512 NV12 camera")
        } else {
            None
        }
    }

    fn reset(&mut self) {
        let _ = self
            .function
            .set_interface(u8::from(self.streaming_interface), 0);
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let request = *xfer.request();
        if !self.accepts_control(&request) {
            return;
        }
        let request_type = 0xa0 | request.request_type as u8 | request.recipient as u8;
        match self.function.control(
            request_type,
            request.request,
            request.value,
            request.index,
            request.length,
            &[],
        ) {
            Ok(ControlResponse::Data(data)) => {
                let _ = xfer.accept_with(data);
            }
            Ok(ControlResponse::Ack) | Err(_) => {
                let _ = xfer.reject();
            }
        }
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let request = *xfer.request();
        if !self.accepts_control(&request) {
            return;
        }
        let request_type = 0x20 | request.request_type as u8 | request.recipient as u8;
        let accepted = matches!(
            self.function.control(
                request_type,
                request.request,
                request.value,
                request.index,
                request.length,
                xfer.data(),
            ),
            Ok(ControlResponse::Ack)
        );
        if accepted {
            let _ = xfer.accept();
        } else {
            let _ = xfer.reject();
        }
    }

    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        if interface == self.streaming_interface {
            Some(self.function.alternate_setting())
        } else if interface == self.control_interface {
            Some(0)
        } else {
            None
        }
    }

    fn set_alt_setting(&mut self, interface: InterfaceNumber, alternative: u8) -> bool {
        if interface != self.streaming_interface {
            return false;
        }
        let Ok(()) = self
            .function
            .set_interface(u8::from(interface), alternative)
        else {
            return false;
        };
        if alternative == 1 {
            if let Some(marker) = self.function.start_stream_packet() {
                let _ = self.endpoint.write(marker);
            }
        }
        true
    }

    fn endpoint_in_complete(&mut self, address: EndpointAddress) {
        if address == self.endpoint.address() {
            if let Some(packet) = self.function.next_in_packet() {
                let _ = self.endpoint.write(packet);
            }
        }
    }
}

pub fn usb_bus() -> UsbBusAllocator<synopsys_usb_otg::UsbBus<F407UsbHs>> {
    static mut EP_MEMORY: [u32; 32] = [0; 32];
    let memory = unsafe { &mut *core::ptr::addr_of_mut!(EP_MEMORY) };
    synopsys_usb_otg::UsbBus::new(F407UsbHs, memory)
}
