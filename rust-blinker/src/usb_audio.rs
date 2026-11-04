//! Descriptor-only USB Audio Class 1.0 capture function.
//!
//! This exposes a mono, 48 kHz, 16-bit PCM microphone endpoint so the host can
//! enumerate the audio function alongside UVC. Audio packet production is not
//! implemented yet; the isochronous IN endpoint is intentionally left idle.

use usb_device::bus::{InterfaceNumber, StringIndex, UsbBus, UsbBusAllocator};
use usb_device::class::UsbClass;
use usb_device::descriptor::DescriptorWriter;
use usb_device::endpoint::{
    EndpointAddress, EndpointIn, EndpointType, IsochronousSynchronizationType,
    IsochronousUsageType,
};
use usb_device::Result;

const AUDIO_CLASS: u8 = 0x01;
const AUDIO_CONTROL_SUBCLASS: u8 = 0x01;
const AUDIO_STREAMING_SUBCLASS: u8 = 0x02;
const AUDIO_ENDPOINT_IN: u8 = 0x82;
const AUDIO_PACKET_BYTES: u16 = 12; // 48 kHz * 2 bytes/sample / 8000 microframes/s

/// USB Audio Class 1.0 capture function (device-to-host microphone).
pub struct AudioUsbClass<'a, B: UsbBus> {
    control_interface: InterfaceNumber,
    streaming_interface: InterfaceNumber,
    function_string: StringIndex,
    endpoint: EndpointIn<'a, B>,
    streaming_alt: u8,
}

impl<'a, B: UsbBus> AudioUsbClass<'a, B> {
    pub fn new(allocator: &'a UsbBusAllocator<B>) -> Self {
        let control_interface = allocator.interface();
        let streaming_interface = allocator.interface();
        let function_string = allocator.string();
        let endpoint = allocator
            .alloc(
                Some(EndpointAddress::from(AUDIO_ENDPOINT_IN)),
                EndpointType::Isochronous {
                    synchronization: IsochronousSynchronizationType::Asynchronous,
                    usage: IsochronousUsageType::Data,
                },
                AUDIO_PACKET_BYTES,
                1,
            )
            .expect("failed to allocate USB Audio isochronous IN endpoint");

        Self {
            control_interface,
            streaming_interface,
            function_string,
            endpoint,
            streaming_alt: 0,
        }
    }

    fn write_audio_control_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(
            self.control_interface,
            AUDIO_CLASS,
            AUDIO_CONTROL_SUBCLASS,
            0,
        )?;

        // UAC1 AudioControl header. The class-specific AC descriptor collection
        // is 30 bytes: header (9), microphone input terminal (12), and
        // USB-streaming output terminal (9).
        writer.write(
            0x24,
            &[
                1,       // HEADER
                0x00, 0x01, // bcdADC = 1.00
                0x1E, 0x00, // wTotalLength = 30
                1,       // bInCollection
                u8::from(self.streaming_interface),
            ],
        )?;

        // Input Terminal: physical microphone, mono.
        writer.write(
            0x24,
            &[2, 1, 0x01, 0x02, 0, 1, 0x01, 0x00, 0, 0],
        )?;

        // Output Terminal: USB Streaming, fed directly by the microphone.
        writer.write(0x24, &[3, 3, 0x01, 0x01, 0, 1, 0])?;
        Ok(())
    }

    fn write_audio_streaming_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface_alt(
            self.streaming_interface,
            0,
            AUDIO_CLASS,
            AUDIO_STREAMING_SUBCLASS,
            0,
            Some(self.function_string),
        )?;

        writer.interface_alt(
            self.streaming_interface,
            1,
            AUDIO_CLASS,
            AUDIO_STREAMING_SUBCLASS,
            0,
            Some(self.function_string),
        )?;

        // AS_GENERAL: USB-streaming output terminal (ID 3), PCM format.
        // AS_GENERAL: terminal link 3, no delay, PCM format tag 0x0001.
        // The descriptor is 7 bytes total: 2-byte header + 5-byte payload.
        writer.write(0x24, &[1, 3, 0, 1, 0])?;

        // FORMAT_TYPE_I: mono, 16-bit samples, one discrete rate (48 kHz).
        writer.write(0x24, &[2, 1, 1, 2, 16, 1, 0x80, 0xBB, 0x00])?;

        // HS isochronous asynchronous data IN endpoint: 12 bytes per
        // 125-us microframe, corresponding to six mono 16-bit samples.
        // UAC1 requires the standard audio endpoint descriptor to be 9 bytes,
        // not the generic 7-byte USB endpoint descriptor. The final two fields
        // are bRefresh and bSynchAddress; both are zero for this capture stream.
        writer.endpoint_ex(&self.endpoint, |buf| {
            if buf.len() < 2 {
                return Err(usb_device::UsbError::BufferOverflow);
            }
            buf[0] = 0; // bRefresh: no synchronization endpoint
            buf[1] = 0; // bSynchAddress: no synchronization endpoint
            Ok(2)
        })?;

        // Class-specific endpoint descriptor; no endpoint controls advertised.
        writer.write(0x25, &[1, 0, 0, 0, 0])?;
        Ok(())
    }
}

impl<B: UsbBus> UsbClass<B> for AudioUsbClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        // Associate both AudioControl and AudioStreaming interfaces as one
        // function for Windows composite-device driver binding.
        writer.iad(
            self.control_interface,
            2,
            AUDIO_CLASS,
            0,
            0,
            Some(self.function_string),
        )?;
        self.write_audio_control_descriptors(writer)?;
        self.write_audio_streaming_descriptors(writer)
    }

    fn get_string(
        &self,
        index: StringIndex,
        _lang_id: usb_device::descriptor::lang_id::LangID,
    ) -> Option<&str> {
        if index == self.function_string {
            Some("XCore407I Audio")
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.streaming_alt = 0;
    }

    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        if interface == self.control_interface {
            Some(0)
        } else if interface == self.streaming_interface {
            Some(self.streaming_alt)
        } else {
            None
        }
    }

    fn set_alt_setting(&mut self, interface: InterfaceNumber, alternative: u8) -> bool {
        if interface == self.control_interface {
            return alternative == 0;
        }
        if interface != self.streaming_interface || alternative > 1 {
            return false;
        }
        self.streaming_alt = alternative;
        true
    }
}
