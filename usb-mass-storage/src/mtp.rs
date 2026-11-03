//! USB Still Image/PTP transport used by Windows MTP.
//!
//! MTP is carried over the USB Still Image class (06h/01h/01h). The data
//! plane consists of bulk OUT, bulk IN, and interrupt IN endpoints. This
//! module intentionally contains no UVC/isochronous scheduling code.

use usb_device::bus::{InterfaceNumber, StringIndex, UsbBusAllocator};
use usb_device::class::UsbClass;
use usb_device::descriptor::DescriptorWriter;
use usb_device::endpoint::{EndpointIn, EndpointOut, EndpointType};
use usb_device::{Result, UsbError};

/// USB Still Image class / PTP subclass / PTP protocol.
pub const STILL_IMAGE_CLASS: u8 = 0x06;
pub const PTP_SUBCLASS: u8 = 0x01;
pub const PTP_PROTOCOL: u8 = 0x01;

/// USB 2.0 High-Speed maximum packet sizes.
pub const BULK_MAX_PACKET_SIZE: u16 = 512;
pub const EVENT_MAX_PACKET_SIZE: u16 = 64;
pub const EVENT_INTERVAL: u8 = 4;

pub const CONTAINER_HEADER_LEN: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ContainerType {
    Command = 1,
    Data = 2,
    Response = 3,
    Event = 4,
}

impl TryFrom<u16> for ContainerType {
    type Error = MtpError;

    fn try_from(value: u16) -> core::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Command),
            2 => Ok(Self::Data),
            3 => Ok(Self::Response),
            4 => Ok(Self::Event),
            _ => Err(MtpError::InvalidContainerType),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MtpError {
    InvalidContainerType,
    InvalidContainerLength,
    TooManyParameters,
    OutputBufferTooSmall,
}

/// Common 12-byte PTP/MTP container header. All multi-byte fields are little-endian.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContainerHeader {
    pub length: u32,
    pub kind: ContainerType,
    pub code: u16,
    pub transaction_id: u32,
}

impl ContainerHeader {
    pub fn decode(bytes: &[u8]) -> core::result::Result<Self, MtpError> {
        if bytes.len() < CONTAINER_HEADER_LEN {
            return Err(MtpError::InvalidContainerLength);
        }
        let length = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        if length < CONTAINER_HEADER_LEN as u32 {
            return Err(MtpError::InvalidContainerLength);
        }
        Ok(Self {
            length,
            kind: ContainerType::try_from(u16::from_le_bytes([bytes[4], bytes[5]]))?,
            code: u16::from_le_bytes([bytes[6], bytes[7]]),
            transaction_id: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        })
    }

    pub fn encode(&self, out: &mut [u8]) -> core::result::Result<(), MtpError> {
        if out.len() < CONTAINER_HEADER_LEN {
            return Err(MtpError::OutputBufferTooSmall);
        }
        out[0..4].copy_from_slice(&self.length.to_le_bytes());
        out[4..6].copy_from_slice(&(self.kind as u16).to_le_bytes());
        out[6..8].copy_from_slice(&self.code.to_le_bytes());
        out[8..12].copy_from_slice(&self.transaction_id.to_le_bytes());
        Ok(())
    }
}

/// PTP/MTP operation codes used by the initial Windows-compatible subset.
pub mod operation {
    pub const GET_DEVICE_INFO: u16 = 0x1001;
    pub const OPEN_SESSION: u16 = 0x1002;
    pub const CLOSE_SESSION: u16 = 0x1003;
    pub const GET_STORAGE_IDS: u16 = 0x1004;
    pub const GET_STORAGE_INFO: u16 = 0x1005;
    pub const GET_NUM_OBJECTS: u16 = 0x1006;
    pub const GET_OBJECT_HANDLES: u16 = 0x1007;
    pub const GET_OBJECT_INFO: u16 = 0x1008;
    pub const GET_OBJECT: u16 = 0x1009;
    pub const DELETE_OBJECT: u16 = 0x100b;
    pub const SEND_OBJECT_INFO: u16 = 0x100c;
    pub const SEND_OBJECT: u16 = 0x100d;
    pub const GET_DEVICE_PROP_DESC: u16 = 0x1014;
    pub const GET_DEVICE_PROP_VALUE: u16 = 0x1015;
    pub const GET_OBJECT_PROPS_SUPPORTED: u16 = 0x9801;
    pub const GET_OBJECT_PROP_DESC: u16 = 0x9802;
    pub const GET_OBJECT_PROP_VALUE: u16 = 0x9803;
    pub const GET_OBJECT_PROP_LIST: u16 = 0x9805;
    pub const GET_OBJECT_REFERENCES: u16 = 0x9810;
}

pub mod response {
    pub const OK: u16 = 0x2001;
    pub const GENERAL_ERROR: u16 = 0x2002;
    pub const SESSION_NOT_OPEN: u16 = 0x2003;
    pub const INVALID_TRANSACTION_ID: u16 = 0x2004;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x2005;
    pub const INCOMPLETE_TRANSFER: u16 = 0x2007;
    pub const INVALID_STORAGE_ID: u16 = 0x2008;
    pub const INVALID_OBJECT_HANDLE: u16 = 0x2009;
    pub const STORE_FULL: u16 = 0x200c;
    pub const ACCESS_DENIED: u16 = 0x200f;
    pub const SESSION_ALREADY_OPEN: u16 = 0x201e;
    pub const INVALID_PARAMETER: u16 = 0x201d;
}

/// Encode a response container, including up to five 32-bit response parameters.
pub fn encode_response(
    transaction_id: u32,
    code: u16,
    parameters: &[u32],
    out: &mut [u8],
) -> core::result::Result<usize, MtpError> {
    if parameters.len() > 5 {
        return Err(MtpError::TooManyParameters);
    }
    let length = CONTAINER_HEADER_LEN + parameters.len() * 4;
    if out.len() < length {
        return Err(MtpError::OutputBufferTooSmall);
    }
    ContainerHeader {
        length: length as u32,
        kind: ContainerType::Response,
        code,
        transaction_id,
    }
    .encode(out)?;
    for (index, value) in parameters.iter().enumerate() {
        let offset = CONTAINER_HEADER_LEN + index * 4;
        out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(length)
}

/// USB PTP transport endpoints and interface descriptor.
///
/// The class deliberately exposes packet-level endpoint access. MTP data
/// containers can be larger than one USB packet and must be streamed by the
/// protocol engine without allocating a whole-file RAM buffer.
pub struct MtpUsbClass<'a, B: usb_device::bus::UsbBus> {
    interface: InterfaceNumber,
    interface_string: StringIndex,
    bulk_out: EndpointOut<'a, B>,
    bulk_in: EndpointIn<'a, B>,
    event_in: EndpointIn<'a, B>,
}

impl<'a, B: usb_device::bus::UsbBus> MtpUsbClass<'a, B> {
    pub fn new(allocator: &'a UsbBusAllocator<B>) -> Self {
        let interface = allocator.interface();
        let interface_string = allocator.string();
        let bulk_out = allocator
            .alloc(None, EndpointType::Bulk, BULK_MAX_PACKET_SIZE, 0)
            .expect("MTP bulk OUT endpoint allocation failed");
        let bulk_in = allocator
            .alloc(None, EndpointType::Bulk, BULK_MAX_PACKET_SIZE, 0)
            .expect("MTP bulk IN endpoint allocation failed");
        let event_in = allocator
            .alloc(None, EndpointType::Interrupt, EVENT_MAX_PACKET_SIZE, EVENT_INTERVAL)
            .expect("MTP event IN endpoint allocation failed");

        Self { interface, interface_string, bulk_out, bulk_in, event_in }
    }

    pub fn bulk_out(&self) -> &EndpointOut<'a, B> { &self.bulk_out }
    pub fn bulk_in(&self) -> &EndpointIn<'a, B> { &self.bulk_in }
    pub fn event_in(&self) -> &EndpointIn<'a, B> { &self.event_in }
    pub fn interface(&self) -> InterfaceNumber { self.interface }

    /// Clear endpoint stalls after a PTP Cancel/Reset recovery sequence.
    pub fn clear_transport_stalls(&self) {
        self.bulk_in.unstall();
        self.bulk_out.unstall();
    }
}

impl<B: usb_device::bus::UsbBus> UsbClass<B> for MtpUsbClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        // PTP class descriptors are recognized by Windows' inbox WPD/MTP driver.
        writer.interface(self.interface, STILL_IMAGE_CLASS, PTP_SUBCLASS, PTP_PROTOCOL)?;
        writer.endpoint(&self.bulk_out)?;
        writer.endpoint(&self.bulk_in)?;
        writer.endpoint(&self.event_in)?;
        Ok(())
    }

    fn get_string(
        &self,
        index: StringIndex,
        _lang_id: usb_device::descriptor::lang_id::LangID,
    ) -> Option<&str> {
        if index == self.interface_string { Some("LittleFS file transfer (MTP)") } else { None }
    }

    fn reset(&mut self) {
        self.clear_transport_stalls();
    }
}

/// Minimal storage contract for the MTP object layer. A LittleFS-backed
/// implementation will map object handles to paths and stream file contents
/// through caller-provided buffers; it must never expose NAND sectors to MTP.
pub trait MtpStorage {
    fn storage_id(&self) -> u32;
    fn capacity_bytes(&self) -> u64;
    fn free_bytes(&self) -> u64;
    fn object_count(&self) -> u32;
    fn read_object(&mut self, handle: u32, offset: u64, out: &mut [u8]) -> Result<usize, MtpStorageError>;
    fn write_object(&mut self, handle: u32, offset: u64, data: &[u8]) -> Result<usize, MtpStorageError>;
    fn delete_object(&mut self, handle: u32) -> Result<(), MtpStorageError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MtpStorageError {
    NotFound,
    ReadOnly,
    NoSpace,
    Io,
}
