//! USB Still Image/PTP transport used by Windows MTP.
//!
//! MTP is carried over the USB Still Image class (06h/01h/01h). The data
//! plane consists of bulk OUT, bulk IN, and interrupt IN endpoints. This
//! module intentionally contains no UVC/isochronous scheduling code.

use usb_device::bus::{InterfaceNumber, StringIndex, UsbBusAllocator};
use usb_device::class::UsbClass;
use usb_device::descriptor::DescriptorWriter;
use usb_device::endpoint::{EndpointIn, EndpointOut, EndpointType};
use usb_device::Result;

/// USB Still Image class / PTP subclass / PTP protocol.
pub const STILL_IMAGE_CLASS: u8 = 0x06;
pub const PTP_SUBCLASS: u8 = 0x01;
pub const PTP_PROTOCOL: u8 = 0x01;

/// USB 2.0 High-Speed maximum packet sizes.
pub const BULK_MAX_PACKET_SIZE: u16 = 512;
pub const EVENT_MAX_PACKET_SIZE: u16 = 64;
pub const EVENT_INTERVAL: u8 = 4;

pub const CONTAINER_HEADER_LEN: usize = 12;
/// Temporary read-only placeholder store, replaced by the mounted LittleFS backend.
const PLACEHOLDER_STORAGE_ID: u32 = 0x0001_0001;
const DEMO_OBJECT_HANDLE: u32 = 1;
const DEMO_OBJECT_NAME: &str = "README.txt";
const DEMO_OBJECT: &[u8] = b"XCore407I MTP test file.\r\nThis is embedded read-only demo data, not NAND/LittleFS content.\r\n";

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
    tx: [u8; 256],
    tx_len: usize,
    pending_response: Option<(u16, u32)>,
    session_open: bool,
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

        Self {
            interface,
            interface_string,
            bulk_out,
            bulk_in,
            event_in,
            tx: [0; 256],
            tx_len: 0,
            pending_response: None,
            session_open: false,
        }
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


impl<'a, B: usb_device::bus::UsbBus> MtpUsbClass<'a, B> {
    fn append_u16(out: &mut [u8], pos: &mut usize, value: u16) {
        out[*pos..*pos + 2].copy_from_slice(&value.to_le_bytes());
        *pos += 2;
    }

    fn append_u32(out: &mut [u8], pos: &mut usize, value: u32) {
        out[*pos..*pos + 4].copy_from_slice(&value.to_le_bytes());
        *pos += 4;
    }

    fn append_ptp_string(out: &mut [u8], pos: &mut usize, value: &str) {
        let units = value.encode_utf16().count();
        out[*pos] = (units + 1) as u8;
        *pos += 1;
        for unit in value.encode_utf16() {
            out[*pos..*pos + 2].copy_from_slice(&unit.to_le_bytes());
            *pos += 2;
        }
        out[*pos..*pos + 2].copy_from_slice(&0u16.to_le_bytes());
        *pos += 2;
    }

    fn queue_data(&mut self, code: u16, transaction_id: u32, payload: &[u8], response_code: u16) {
        let length = CONTAINER_HEADER_LEN + payload.len();
        self.tx[..4].copy_from_slice(&(length as u32).to_le_bytes());
        self.tx[4..6].copy_from_slice(&(ContainerType::Data as u16).to_le_bytes());
        self.tx[6..8].copy_from_slice(&code.to_le_bytes());
        self.tx[8..12].copy_from_slice(&transaction_id.to_le_bytes());
        self.tx[12..length].copy_from_slice(payload);
        self.tx_len = length;
        self.pending_response = Some((response_code, transaction_id));
        let _ = self.bulk_in.write(&self.tx[..self.tx_len]);
    }

    fn queue_response(&mut self, code: u16, transaction_id: u32) {
        self.queue_response_with_params(code, transaction_id, &[]);
    }

    fn queue_response_with_params(&mut self, code: u16, transaction_id: u32, params: &[u32]) {
        let mut response_bytes = [0u8; 32];
        if let Ok(length) = encode_response(transaction_id, code, params, &mut response_bytes) {
            let _ = self.bulk_in.write(&response_bytes[..length]);
        }
        self.pending_response = None;
        self.tx_len = 0;
    }

    fn device_info(&mut self, transaction_id: u32) {
        let mut data = [0u8; 240];
        let mut pos = 0;
        Self::append_u16(&mut data, &mut pos, 100); // StandardVersion
        Self::append_u32(&mut data, &mut pos, 6); // Microsoft MTP vendor extension
        Self::append_u16(&mut data, &mut pos, 100);
        Self::append_ptp_string(&mut data, &mut pos, "microsoft.com: 1.0;");
        Self::append_u16(&mut data, &mut pos, 0); // FunctionalMode

        let operations: [u16; 11] = [
            operation::GET_DEVICE_INFO,
            operation::OPEN_SESSION,
            operation::CLOSE_SESSION,
            operation::GET_STORAGE_IDS,
            operation::GET_STORAGE_INFO,
            operation::GET_NUM_OBJECTS,
            operation::GET_OBJECT_HANDLES,
            operation::GET_OBJECT_INFO,
            operation::GET_OBJECT,
            operation::GET_OBJECT_PROPS_SUPPORTED,
            operation::GET_OBJECT_PROP_LIST,
        ];
        Self::append_u32(&mut data, &mut pos, operations.len() as u32);
        for value in operations { Self::append_u16(&mut data, &mut pos, value); }

        Self::append_u32(&mut data, &mut pos, 0); // EventsSupported
        Self::append_u32(&mut data, &mut pos, 0); // DevicePropertiesSupported
        Self::append_u32(&mut data, &mut pos, 0); // CaptureFormats
        Self::append_u32(&mut data, &mut pos, 1); // PlaybackFormats
        Self::append_u16(&mut data, &mut pos, 0x3004); // Text

        Self::append_ptp_string(&mut data, &mut pos, "OpenAI XCore407I");
        Self::append_ptp_string(&mut data, &mut pos, "XCore407I");
        Self::append_ptp_string(&mut data, &mut pos, "0.1");
        Self::append_ptp_string(&mut data, &mut pos, "XCORE407I-001");

        self.queue_data(operation::GET_DEVICE_INFO, transaction_id, &data[..pos], response::OK);
    }


    fn storage_info(&mut self, transaction_id: u32, storage_id: u32) {
        if storage_id != PLACEHOLDER_STORAGE_ID {
            self.queue_response(response::INVALID_STORAGE_ID, transaction_id);
            return;
        }
        // Read-only embedded demo store; NAND/LittleFS is not mounted.
        let mut data = [0u8; 96];
        let mut pos = 0;
        Self::append_u16(&mut data, &mut pos, 0x0001); // Fixed ROM
        Self::append_u16(&mut data, &mut pos, 0x0002); // Hierarchical filesystem
        Self::append_u16(&mut data, &mut pos, 0x0001); // Read-only
        Self::append_u64(&mut data, &mut pos, 64 * 1024);
        Self::append_u64(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_ptp_string(&mut data, &mut pos, "Data");
        Self::append_ptp_string(&mut data, &mut pos, "XCORE407I");
        self.queue_data(operation::GET_STORAGE_INFO, transaction_id, &data[..pos], response::OK);
    }

    fn append_u64(out: &mut [u8], pos: &mut usize, value: u64) {
        out[*pos..*pos + 8].copy_from_slice(&value.to_le_bytes());
        *pos += 8;
    }

    fn object_info(&mut self, transaction_id: u32, storage_id: u32, handle: u32) {
        if storage_id != PLACEHOLDER_STORAGE_ID {
            self.queue_response(response::INVALID_STORAGE_ID, transaction_id);
            return;
        }
        if handle != DEMO_OBJECT_HANDLE {
            self.queue_response(response::INVALID_OBJECT_HANDLE, transaction_id);
            return;
        }

        let mut data = [0u8; 160];
        let mut pos = 0;
        Self::append_u32(&mut data, &mut pos, PLACEHOLDER_STORAGE_ID);
        Self::append_u16(&mut data, &mut pos, 0x3004); // Text
        Self::append_u16(&mut data, &mut pos, 1); // Read-only
        Self::append_u32(&mut data, &mut pos, DEMO_OBJECT.len() as u32);
        Self::append_u16(&mut data, &mut pos, 0); // ThumbFormat
        Self::append_u32(&mut data, &mut pos, 0); // ThumbCompressedSize
        Self::append_u32(&mut data, &mut pos, 0); // ThumbPixWidth
        Self::append_u32(&mut data, &mut pos, 0); // ThumbPixHeight
        Self::append_u32(&mut data, &mut pos, 0); // ImagePixWidth
        Self::append_u32(&mut data, &mut pos, 0); // ImagePixHeight
        Self::append_u32(&mut data, &mut pos, 0); // ImageBitDepth
        Self::append_u32(&mut data, &mut pos, 0xffff_ffff); // ParentObject (root)
        Self::append_u16(&mut data, &mut pos, 0); // AssociationType
        Self::append_u32(&mut data, &mut pos, 0); // AssociationDesc
        Self::append_u32(&mut data, &mut pos, 0); // SequenceNumber
        Self::append_ptp_string(&mut data, &mut pos, DEMO_OBJECT_NAME);
        Self::append_ptp_string(&mut data, &mut pos, "");
        Self::append_ptp_string(&mut data, &mut pos, "");
        Self::append_ptp_string(&mut data, &mut pos, "");
        self.queue_data(operation::GET_OBJECT_INFO, transaction_id, &data[..pos], response::OK);
    }

    fn object_props_supported(&mut self, transaction_id: u32, format: u32) {
        if format != 0 && format != 0x3004 {
            self.queue_response(response::OK, transaction_id);
            return;
        }
        // StorageID, ObjectFormat, ProtectionStatus, ObjectSize, ObjectFileName,
        // DateModified, ParentObject.
        let props: [u16; 7] = [0xdc01, 0xdc02, 0xdc03, 0xdc04, 0xdc07, 0xdc09, 0xdc0b];
        let mut data = [0u8; 32];
        data[..4].copy_from_slice(&(props.len() as u32).to_le_bytes());
        for (i, prop) in props.iter().enumerate() {
            let at = 4 + i * 2;
            data[at..at + 2].copy_from_slice(&prop.to_le_bytes());
        }
        self.queue_data(operation::GET_OBJECT_PROPS_SUPPORTED, transaction_id, &data[..18], response::OK);
    }

    fn append_prop_u16(out: &mut [u8], pos: &mut usize, handle: u32, code: u16, value: u16) {
        Self::append_u32(out, pos, handle);
        Self::append_u16(out, pos, code);
        Self::append_u16(out, pos, 0x0004); // UINT16
        Self::append_u16(out, pos, value);
    }

    fn append_prop_u32(out: &mut [u8], pos: &mut usize, handle: u32, code: u16, value: u32) {
        Self::append_u32(out, pos, handle);
        Self::append_u16(out, pos, code);
        Self::append_u16(out, pos, 0x0006); // UINT32
        Self::append_u32(out, pos, value);
    }

    fn append_prop_u64(out: &mut [u8], pos: &mut usize, handle: u32, code: u16, value: u64) {
        Self::append_u32(out, pos, handle);
        Self::append_u16(out, pos, code);
        Self::append_u16(out, pos, 0x0008); // UINT64
        Self::append_u64(out, pos, value);
    }

    fn append_prop_string(out: &mut [u8], pos: &mut usize, handle: u32, code: u16, value: &str) {
        Self::append_u32(out, pos, handle);
        Self::append_u16(out, pos, code);
        Self::append_u16(out, pos, 0xffff); // STRING
        Self::append_ptp_string(out, pos, value);
    }

    fn object_prop_list(&mut self, transaction_id: u32, handle: u32) {
        if handle != 0 && handle != DEMO_OBJECT_HANDLE && handle != 0xffff_ffff {
            self.queue_response(response::INVALID_OBJECT_HANDLE, transaction_id);
            return;
        }
        let mut data = [0u8; 256];
        let mut pos = 0;
        // A single object and seven standard properties.
        Self::append_u32(&mut data, &mut pos, 7);
        let h = DEMO_OBJECT_HANDLE;
        Self::append_prop_u32(&mut data, &mut pos, h, 0xdc01, PLACEHOLDER_STORAGE_ID);
        Self::append_prop_u16(&mut data, &mut pos, h, 0xdc02, 0x3004);
        Self::append_prop_u16(&mut data, &mut pos, h, 0xdc03, 1);
        Self::append_prop_u64(&mut data, &mut pos, h, 0xdc04, DEMO_OBJECT.len() as u64);
        Self::append_prop_string(&mut data, &mut pos, h, 0xdc07, DEMO_OBJECT_NAME);
        Self::append_prop_string(&mut data, &mut pos, h, 0xdc09, "20261102T000000");
        Self::append_prop_u32(&mut data, &mut pos, h, 0xdc0b, 0);
        self.queue_data(operation::GET_OBJECT_PROP_LIST, transaction_id, &data[..pos], response::OK);
    }

    fn handle_command(&mut self, packet: &[u8]) {
        let Ok(header) = ContainerHeader::decode(packet) else { return; };
        if header.kind != ContainerType::Command
            || header.length as usize > packet.len()
            || header.length < CONTAINER_HEADER_LEN as u32
        {
            return;
        }
        let params = &packet[CONTAINER_HEADER_LEN..header.length as usize];
        let parameter = |index: usize| -> Option<u32> {
            let start = index * 4;
            if params.len() < start + 4 { None } else {
                Some(u32::from_le_bytes(params[start..start + 4].try_into().unwrap()))
            }
        };

        match header.code {
            operation::GET_DEVICE_INFO => self.device_info(header.transaction_id),
            operation::OPEN_SESSION => {
                if self.session_open {
                    self.queue_response(response::SESSION_ALREADY_OPEN, header.transaction_id);
                } else if parameter(0).unwrap_or(0) == 0 || params.len() != 4 {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                } else {
                    self.session_open = true;
                    self.queue_response(response::OK, header.transaction_id);
                }
            }
            operation::CLOSE_SESSION => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else {
                    self.session_open = false;
                    self.queue_response(response::OK, header.transaction_id);
                }
            }
            operation::GET_STORAGE_IDS => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else {
                    let mut ids = [0u8; 8];
                    ids[..4].copy_from_slice(&1u32.to_le_bytes());
                    ids[4..8].copy_from_slice(&PLACEHOLDER_STORAGE_ID.to_le_bytes());
                    self.queue_data(operation::GET_STORAGE_IDS, header.transaction_id, &ids, response::OK);
                }
            }
            operation::GET_STORAGE_INFO => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if let Some(storage_id) = parameter(0) {
                    self.storage_info(header.transaction_id, storage_id);
                } else {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                }
            }
            operation::GET_NUM_OBJECTS | operation::GET_OBJECT_HANDLES => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if parameter(0) != Some(PLACEHOLDER_STORAGE_ID)
                    && parameter(0) != Some(0xffff_ffff)
                {
                    self.queue_response(response::INVALID_STORAGE_ID, header.transaction_id);
                } else if header.code == operation::GET_NUM_OBJECTS {
                    // GetNumObjects returns the count as a response parameter.
                    self.queue_response_with_params(response::OK, header.transaction_id, &[1]);
                } else {
                    // One embedded test object at the storage root.
                    let mut handles = [0u8; 8];
                    handles[..4].copy_from_slice(&1u32.to_le_bytes());
                    handles[4..8].copy_from_slice(&DEMO_OBJECT_HANDLE.to_le_bytes());
                    self.queue_data(header.code, header.transaction_id, &handles, response::OK);
                }
            }
            operation::GET_OBJECT_INFO => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if let (Some(storage_id), Some(handle)) = (parameter(0), parameter(1)) {
                    self.object_info(header.transaction_id, storage_id, handle);
                } else {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                }
            }
            operation::GET_OBJECT_PROPS_SUPPORTED => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if let Some(format) = parameter(0) {
                    self.object_props_supported(header.transaction_id, format);
                } else {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                }
            }
            operation::GET_OBJECT_PROP_LIST => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if let Some(handle) = parameter(0) {
                    self.object_prop_list(header.transaction_id, handle);
                } else {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                }
            }
            operation::GET_OBJECT => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if parameter(0) != Some(DEMO_OBJECT_HANDLE) {
                    self.queue_response(response::INVALID_OBJECT_HANDLE, header.transaction_id);
                } else {
                    self.queue_data(operation::GET_OBJECT, header.transaction_id, DEMO_OBJECT, response::OK);
                }
            }
            _ => self.queue_response(response::OPERATION_NOT_SUPPORTED, header.transaction_id),
        }
    }
}

impl<B: usb_device::bus::UsbBus> UsbClass<B> for MtpUsbClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
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
        if index == self.interface_string { Some("XCore407I MTP data") } else { None }
    }

    fn endpoint_out(&mut self, address: usb_device::endpoint::EndpointAddress) {
        if address != self.bulk_out.address() { return; }
        let mut packet = [0u8; BULK_MAX_PACKET_SIZE as usize];
        if let Ok(length) = self.bulk_out.read(&mut packet) {
            self.handle_command(&packet[..length]);
        }
    }

    fn endpoint_in_complete(&mut self, address: usb_device::endpoint::EndpointAddress) {
        if address != self.bulk_in.address() { return; }
        if let Some((code, transaction_id)) = self.pending_response {
            self.queue_response(code, transaction_id);
        }
    }

    fn reset(&mut self) {
        self.clear_transport_stalls();
        self.session_open = false;
        self.pending_response = None;
        self.tx_len = 0;
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
    fn read_object(&mut self, handle: u32, offset: u64, out: &mut [u8]) -> core::result::Result<usize, MtpStorageError>;
    fn write_object(&mut self, handle: u32, offset: u64, data: &[u8]) -> core::result::Result<usize, MtpStorageError>;
    fn delete_object(&mut self, handle: u32) -> core::result::Result<(), MtpStorageError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MtpStorageError {
    NotFound,
    ReadOnly,
    NoSpace,
    Io,
}
