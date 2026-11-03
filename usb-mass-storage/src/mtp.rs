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

/// Live transport counters for debugger inspection. They intentionally use
/// atomics because callbacks run from the TIM2 USB polling interrupt.
pub mod diagnostics {
    use core::sync::atomic::AtomicU32;
    pub static OUT_PACKETS: AtomicU32 = AtomicU32::new(0);
    pub static OUT_INVALID: AtomicU32 = AtomicU32::new(0);
    pub static COMMANDS_ACCEPTED: AtomicU32 = AtomicU32::new(0);
    pub static COMMANDS_DEFERRED: AtomicU32 = AtomicU32::new(0);
    pub static IN_SUBMIT_OK: AtomicU32 = AtomicU32::new(0);
    pub static IN_SUBMIT_WOULDBLOCK: AtomicU32 = AtomicU32::new(0);
    pub static IN_COMPLETE: AtomicU32 = AtomicU32::new(0);
    pub static USB_RESETS: AtomicU32 = AtomicU32::new(0);
    pub static PENDING_COMMAND_OVERFLOW: AtomicU32 = AtomicU32::new(0);
    pub static DATA_IN_COMPLETED: AtomicU32 = AtomicU32::new(0);
    pub static RESPONSE_IN_COMPLETED: AtomicU32 = AtomicU32::new(0);
    pub static CONTROL_GET_STATUS: AtomicU32 = AtomicU32::new(0);
    pub static CONTROL_DEVICE_RESET: AtomicU32 = AtomicU32::new(0);

    fn append(out: &mut [u8], pos: &mut usize, bytes: &[u8]) {
        let count = bytes.len().min(out.len().saturating_sub(*pos));
        out[*pos..*pos + count].copy_from_slice(&bytes[..count]);
        *pos += count;
    }

    fn append_counter(out: &mut [u8], pos: &mut usize, label: &[u8], value: u32) {
        append(out, pos, label);
        let mut digits = [0u8; 10];
        let mut n = value;
        let mut count = 0;
        loop {
            digits[count] = b'0' + (n % 10) as u8;
            count += 1;
            n /= 10;
            if n == 0 { break; }
        }
        while count != 0 {
            count -= 1;
            append(out, pos, &digits[count..count + 1]);
        }
        append(out, pos, b"\r\n");
    }

    /// Create a fixed-size text snapshot suitable for a virtual MTP object.
    pub fn write_status(out: &mut [u8]) -> usize {
        use core::sync::atomic::Ordering;
        let mut pos = 0;
        append(out, &mut pos, b"XCore407I MTP transport diagnostics\r\n");
        append_counter(out, &mut pos, b"OUT_PACKETS: ", OUT_PACKETS.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"OUT_INVALID: ", OUT_INVALID.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"COMMANDS_ACCEPTED: ", COMMANDS_ACCEPTED.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"COMMANDS_DEFERRED: ", COMMANDS_DEFERRED.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"IN_SUBMIT_OK: ", IN_SUBMIT_OK.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"IN_SUBMIT_WOULDBLOCK: ", IN_SUBMIT_WOULDBLOCK.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"IN_COMPLETE: ", IN_COMPLETE.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"USB_RESETS: ", USB_RESETS.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"PENDING_COMMAND_OVERFLOW: ", PENDING_COMMAND_OVERFLOW.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"DATA_IN_COMPLETED: ", DATA_IN_COMPLETED.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"RESPONSE_IN_COMPLETED: ", RESPONSE_IN_COMPLETED.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"CONTROL_GET_STATUS: ", CONTROL_GET_STATUS.load(Ordering::Relaxed));
        append_counter(out, &mut pos, b"CONTROL_DEVICE_RESET: ", CONTROL_DEVICE_RESET.load(Ordering::Relaxed));
        pos
    }
}

pub const CONTAINER_HEADER_LEN: usize = 12;
/// Temporary read-only placeholder store, replaced by the mounted LittleFS backend.
const PLACEHOLDER_STORAGE_ID: u32 = 0x0001_0001;
const DEMO_OBJECT_HANDLE: u32 = 1;
const DEMO_OBJECT_NAME: &str = "README.txt";
const DEFAULT_README: &[u8] = b"XCore407I NAND diagnostic pending.\r\n";

/// One entry in the bounded root-directory snapshot supplied by the main loop.
#[derive(Clone, Copy)]
pub struct MtpObject {
    pub handle: u32,
    pub name: [u8; 64],
    pub name_len: u8,
    pub size: u32,
    pub is_dir: bool,
}

impl MtpObject {
    pub const EMPTY: Self = Self { handle: 0, name: [0; 64], name_len: 0, size: 0, is_dir: false };
    fn name(&self) -> &str { core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("") }
}
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
    tx: [u8; 512],
    tx_len: usize,
    tx_pending: bool,
    tx_in_flight: bool,
    tx_is_response: bool,
    // MTP commands are serialized: retain one OUT command that arrives while
    // the current transaction's data/response is still being transmitted.
    transaction_active: bool,
    data_transfer_active: bool,
    pending_command: [u8; BULK_MAX_PACKET_SIZE as usize],
    pending_command_len: usize,
    readme: [u8; 192],
    readme_len: usize,
    pending_response: Option<(u16, u32)>,
    pending_get_object: Option<(u32, u32)>,
    pending_delete_object: Option<(u32, u32)>,
    session_open: bool,
    objects: [MtpObject; 17],
    object_count: usize,
    capacity_bytes: u64,
    free_bytes: u64,
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
            tx: [0; 512],
            tx_len: 0,
            tx_pending: false,
            tx_in_flight: false,
            tx_is_response: false,
            transaction_active: false,
            data_transfer_active: false,
            pending_command: [0; BULK_MAX_PACKET_SIZE as usize],
            pending_command_len: 0,
            readme: { let mut bytes = [0u8; 192]; bytes[..DEFAULT_README.len()].copy_from_slice(DEFAULT_README); bytes },
            readme_len: DEFAULT_README.len(),
            pending_response: None,
            pending_get_object: None,
            pending_delete_object: None,
            session_open: false,
            objects: [MtpObject::EMPTY; 17],
            object_count: 0,
            capacity_bytes: 0,
            free_bytes: 0,
        }
    }

    /// Replace the read-only README.txt contents before USB polling.
    pub fn set_readme(&mut self, contents: &[u8]) -> bool {
        if contents.len() > self.readme.len() { return false; }
        self.readme[..contents.len()].copy_from_slice(contents);
        self.readme_len = contents.len();
        true
    }

    /// Replace the bounded root-directory snapshot. Handles remain stable until refreshed.
    pub fn set_objects(&mut self, objects: &[MtpObject]) {
        self.objects = [MtpObject::EMPTY; 17];
        self.object_count = objects.len().min(self.objects.len());
        self.objects[..self.object_count].copy_from_slice(&objects[..self.object_count]);
    }

    pub fn set_storage_stats(&mut self, capacity_bytes: u64, free_bytes: u64) {
        self.capacity_bytes = capacity_bytes;
        self.free_bytes = free_bytes.min(capacity_bytes);
    }

    /// Take a GET_OBJECT request for processing in the main context.
    pub fn take_get_object_request(&mut self) -> Option<(u32, u32)> {
        self.pending_get_object.take()
    }

    /// Take a DELETE_OBJECT request for processing by LittleFS in main context.
    pub fn take_delete_object_request(&mut self) -> Option<(u32, u32)> {
        self.pending_delete_object.take()
    }

    /// True when the previous bulk-IN packet has completed and another chunk
    /// can be queued by the main context.
    pub fn object_tx_ready(&self) -> bool {
        !self.tx_pending && !self.tx_in_flight
    }

    /// Queue one piece of a single MTP GetObject data container. The header is
    /// included only in the first chunk; the final chunk schedules the response.
    pub fn queue_object_data_chunk(
        &mut self,
        transaction_id: u32,
        total_size: u32,
        bytes: &[u8],
        first: bool,
        last: bool,
    ) -> bool {
        let header_len = if first { CONTAINER_HEADER_LEN } else { 0 };
        if bytes.len() + header_len > self.tx.len() || !self.object_tx_ready() {
            return false;
        }
        if first {
            let length = CONTAINER_HEADER_LEN as u32 + total_size;
            self.tx[..4].copy_from_slice(&length.to_le_bytes());
            self.tx[4..6].copy_from_slice(&(ContainerType::Data as u16).to_le_bytes());
            self.tx[6..8].copy_from_slice(&operation::GET_OBJECT.to_le_bytes());
            self.tx[8..12].copy_from_slice(&transaction_id.to_le_bytes());
        }
        self.tx[header_len..header_len + bytes.len()].copy_from_slice(bytes);
        self.tx_len = header_len + bytes.len();
        self.tx_pending = true;
        self.tx_is_response = false;
        if first {
            self.data_transfer_active = true;
        }
        if last {
            self.pending_response = Some((response::OK, transaction_id));
        }
        true
    }

    /// Queue a complete small GetObject data container using the same
    /// single-transfer path as the other MTP data responses.
    pub fn queue_object_data(&mut self, transaction_id: u32, payload: &[u8]) -> bool {
        if payload.len() + CONTAINER_HEADER_LEN > self.tx.len() || !self.object_tx_ready() {
            return false;
        }
        self.queue_data(operation::GET_OBJECT, transaction_id, payload, response::OK);
        true
    }

    /// Queue a response from the main context after a storage-side failure.
    pub fn queue_main_response(&mut self, code: u16, transaction_id: u32) -> bool {
        if !self.object_tx_ready() {
            return false;
        }
        self.data_transfer_active = false;
        self.queue_response(code, transaction_id);
        true
    }

    fn object(&self, handle: u32) -> Option<&MtpObject> {
        self.objects[..self.object_count].iter().find(|object| object.handle == handle)
    }

    pub fn bulk_out(&self) -> &EndpointOut<'a, B> { &self.bulk_out }
    pub fn bulk_in(&self) -> &EndpointIn<'a, B> { &self.bulk_in }
    pub fn event_in(&self) -> &EndpointIn<'a, B> { &self.event_in }
    pub fn interface(&self) -> InterfaceNumber { self.interface }

    /// Clear protocol state after the Still Image class Device Reset request.
    /// This does not reset the USB peripheral or increment the USB bus-reset counter.
    fn reset_protocol_state(&mut self) {
        self.clear_transport_stalls();
        self.session_open = false;
        self.pending_get_object = None;
        self.pending_delete_object = None;
        self.pending_response = None;
        self.pending_command_len = 0;
        self.transaction_active = false;
        self.data_transfer_active = false;
        self.tx_len = 0;
        self.tx_pending = false;
        self.tx_in_flight = false;
        self.tx_is_response = false;
    }

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
        self.tx_pending = true;
        self.tx_is_response = false;
        self.pending_response = Some((response_code, transaction_id));
        self.data_transfer_active = true;
    }

    fn queue_response(&mut self, code: u16, transaction_id: u32) {
        self.queue_response_with_params(code, transaction_id, &[]);
    }

    fn queue_response_with_params(&mut self, code: u16, transaction_id: u32, params: &[u32]) {
        self.pending_response = None;
        if let Ok(length) = encode_response(transaction_id, code, params, &mut self.tx) {
            self.tx_len = length;
            self.tx_pending = true;
            self.tx_is_response = true;
        } else {
            self.tx_len = 0;
            self.tx_is_response = true;
        }
    }

    /// Submit queued MTP data only when the USB IN endpoint accepts it.
    /// A WouldBlock is normal while the previous packet is still in flight;
    /// keep the packet queued and retry on the next USB poll.
    fn transmit_pending(&mut self) {
        if !self.tx_pending || self.tx_in_flight || self.tx_len == 0 {
            return;
        }
        match self.bulk_in.write(&self.tx[..self.tx_len]) {
            Ok(length) if length == self.tx_len => {
                diagnostics::IN_SUBMIT_OK.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                self.tx_pending = false;
                self.tx_in_flight = true;
            }
            Ok(_) | Err(usb_device::UsbError::WouldBlock) => {
                diagnostics::IN_SUBMIT_WOULDBLOCK.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            Err(_) => {
                // Keep the packet queued. A later poll retries transient bus errors.
            }
        }
    }

    fn device_info(&mut self, transaction_id: u32) {
        let mut data = [0u8; 240];
        let mut pos = 0;
        Self::append_u16(&mut data, &mut pos, 100); // StandardVersion
        Self::append_u32(&mut data, &mut pos, 6); // Microsoft MTP vendor extension
        Self::append_u16(&mut data, &mut pos, 100);
        Self::append_ptp_string(&mut data, &mut pos, "microsoft.com: 1.0;");
        Self::append_u16(&mut data, &mut pos, 0); // FunctionalMode

        let operations: [u16; 12] = [
            operation::GET_DEVICE_INFO,
            operation::OPEN_SESSION,
            operation::CLOSE_SESSION,
            operation::GET_STORAGE_IDS,
            operation::GET_STORAGE_INFO,
            operation::GET_NUM_OBJECTS,
            operation::GET_OBJECT_HANDLES,
            operation::GET_OBJECT_INFO,
            operation::GET_OBJECT,
            operation::DELETE_OBJECT,
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
        Self::append_ptp_string(&mut data, &mut pos, "XCORE407I-002");

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
        Self::append_u16(&mut data, &mut pos, 0x0003); // Fixed RAM
        Self::append_u64(&mut data, &mut pos, self.capacity_bytes);
        Self::append_u64(&mut data, &mut pos, self.free_bytes);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_ptp_string(&mut data, &mut pos, "Data");
        Self::append_ptp_string(&mut data, &mut pos, "XCORE407I");
        self.queue_data(operation::GET_STORAGE_INFO, transaction_id, &data[..pos], response::OK);
    }

    fn append_u64(out: &mut [u8], pos: &mut usize, value: u64) {
        out[*pos..*pos + 8].copy_from_slice(&value.to_le_bytes());
        *pos += 8;
    }

    fn object_info(&mut self, transaction_id: u32, handle: u32) {
        let Some(object) = self.object(handle).copied() else {
            self.queue_response(response::INVALID_OBJECT_HANDLE, transaction_id);
            return;
        };
        let mut data = [0u8; 192];
        let mut pos = 0;
        Self::append_u32(&mut data, &mut pos, PLACEHOLDER_STORAGE_ID);
        Self::append_u16(&mut data, &mut pos, if object.is_dir { 0x3001 } else { 0x3000 });
        Self::append_u16(&mut data, &mut pos, 0); // Read/write
        Self::append_u32(&mut data, &mut pos, object.size);
        Self::append_u16(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0xffff_ffff);
        Self::append_u16(&mut data, &mut pos, if object.is_dir { 1 } else { 0 });
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_u32(&mut data, &mut pos, 0);
        Self::append_ptp_string(&mut data, &mut pos, object.name());
        Self::append_ptp_string(&mut data, &mut pos, "");
        Self::append_ptp_string(&mut data, &mut pos, "");
        Self::append_ptp_string(&mut data, &mut pos, "");
        self.queue_data(operation::GET_OBJECT_INFO, transaction_id, &data[..pos], response::OK);
    }

    fn object_props_supported(&mut self, transaction_id: u32, format: u32) {
        if format != 0 && format != 0x3000 && format != 0x3001 {
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
        let selected = if handle == 0 || handle == 0xffff_ffff {
            self.objects[..self.object_count].first().copied()
        } else {
            self.object(handle).copied()
        };
        let Some(object) = selected else {
            self.queue_data(operation::GET_OBJECT_PROP_LIST, transaction_id, &[0, 0, 0, 0], response::OK);
            return;
        };
        let mut data = [0u8; 256];
        let mut pos = 0;
        Self::append_u32(&mut data, &mut pos, 7);
        let h = object.handle;
        Self::append_prop_u32(&mut data, &mut pos, h, 0xdc01, PLACEHOLDER_STORAGE_ID);
        Self::append_prop_u16(&mut data, &mut pos, h, 0xdc02, if object.is_dir { 0x3001 } else { 0x3000 });
        Self::append_prop_u16(&mut data, &mut pos, h, 0xdc03, 0);
        Self::append_prop_u64(&mut data, &mut pos, h, 0xdc04, object.size as u64);
        Self::append_prop_string(&mut data, &mut pos, h, 0xdc07, object.name());
        Self::append_prop_string(&mut data, &mut pos, h, 0xdc09, "20261102T000000");
        Self::append_prop_u32(&mut data, &mut pos, h, 0xdc0b, 0xffff_ffff);
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
                    self.queue_response_with_params(response::OK, header.transaction_id, &[self.object_count as u32]);
                } else {
                    // One embedded test object at the storage root.
                    let mut handles = [0u8; 4 + 17 * 4];
                    handles[..4].copy_from_slice(&(self.object_count as u32).to_le_bytes());
                    for (i, object) in self.objects[..self.object_count].iter().enumerate() {
                        let at = 4 + i * 4;
                        handles[at..at + 4].copy_from_slice(&object.handle.to_le_bytes());
                    }
                    self.queue_data(header.code, header.transaction_id, &handles[..4 + self.object_count * 4], response::OK);
                }
            }
            operation::GET_OBJECT_INFO => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if let Some(handle) = parameter(0) {
                    if params.len() == 4 {
                        self.object_info(header.transaction_id, handle);
                    } else {
                        self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                    }
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
            operation::DELETE_OBJECT => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if (params.len() != 4 && params.len() != 8) {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                } else if let Some(handle) = parameter(0) {
                    if self.object(handle).is_none() {
                        self.queue_response(response::INVALID_OBJECT_HANDLE, header.transaction_id);
                    } else if self.object(handle).map(|object| object.name()) == Some("MTP-README.txt")
                        || self.object(handle).map(|object| object.name()) == Some("MTP-STATUS.txt") {
                        self.queue_response(response::ACCESS_DENIED, header.transaction_id);
                    } else if self.pending_delete_object.is_some() {
                        self.queue_response(response::GENERAL_ERROR, header.transaction_id);
                    } else {
                        self.pending_delete_object = Some((handle, header.transaction_id));
                    }
                } else {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                }
            }
            operation::GET_OBJECT => {
                if !self.session_open {
                    self.queue_response(response::SESSION_NOT_OPEN, header.transaction_id);
                } else if params.len() != 4 {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
                } else if let Some(handle) = parameter(0) {
                    if self.object(handle).is_none() {
                        self.queue_response(response::INVALID_OBJECT_HANDLE, header.transaction_id);
                    } else if self.pending_get_object.is_some() {
                        self.queue_response(response::GENERAL_ERROR, header.transaction_id);
                    } else if self.object(handle).map(|object| object.name()) == Some("MTP-STATUS.txt") {
                        // Snapshot counters directly into the transfer buffer;
                        // this is read-only and does not touch LittleFS.
                        let mut payload = [0u8; 480];
                        let payload_len = diagnostics::write_status(&mut payload);
                        payload[payload_len..].fill(b' ');
                        // ObjectInfo advertises exactly 480 bytes. Send the
                        // complete object, including padding, so the MTP data
                        // container length agrees with the advertised size.
                        self.queue_data(
                            operation::GET_OBJECT,
                            header.transaction_id,
                            &payload,
                            response::OK,
                        );
                    } else if self.object(handle).map(|object| object.name()) == Some("MTP-README.txt") {
                        // Keep one known-good, in-memory object on the original
                        // command/response path. This isolates USB MTP transport
                        // from the deferred LittleFS read path.
                        let mut payload = [0u8; 192];
                        let payload_len = self.readme_len;
                        payload[..payload_len].copy_from_slice(&self.readme[..payload_len]);
                        self.queue_data(
                            operation::GET_OBJECT,
                            header.transaction_id,
                            &payload[..payload_len],
                            response::OK,
                        );
                    } else {
                        self.pending_get_object = Some((handle, header.transaction_id));
                    }
                } else {
                    self.queue_response(response::INVALID_PARAMETER, header.transaction_id);
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

    fn control_in(&mut self, xfer: usb_device::class::ControlIn<'_, '_, '_, B>) {
        let req = *xfer.request();
        if req.request_type != usb_device::control::RequestType::Class
            || req.recipient != usb_device::control::Recipient::Interface
            || req.index > u8::MAX as u16
            || u8::from(self.interface) != req.index as u8
            || req.value != 0
        {
            return;
        }

        // USB Still Image class request 0x67: Get Device Status.
        // The returned structure is { uint16 length, uint16 PTP response code }.
        if req.request == 0x67 && req.length >= 4 {
            diagnostics::CONTROL_GET_STATUS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let code = if self.transaction_active
                || self.tx_pending
                || self.tx_in_flight
                || self.pending_get_object.is_some()
                || self.pending_delete_object.is_some()
            {
                0x2019u16 // Device Busy
            } else {
                response::OK
            };
            let status = [4u8, 0, code as u8, (code >> 8) as u8];
            let _ = xfer.accept_with(&status);
        }
    }

    fn control_out(&mut self, xfer: usb_device::class::ControlOut<'_, '_, '_, B>) {
        let req = *xfer.request();
        if req.request_type != usb_device::control::RequestType::Class
            || req.recipient != usb_device::control::Recipient::Interface
            || req.index > u8::MAX as u16
            || u8::from(self.interface) != req.index as u8
            || req.value != 0
        {
            return;
        }

        // USB Still Image class request 0x66: Device Reset. This is a
        // protocol-level reset, not a USB bus reset.
        if req.request == 0x66 && req.length == 0 {
            if xfer.accept().is_ok() {
                diagnostics::CONTROL_DEVICE_RESET.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                self.reset_protocol_state();
            }
        }
    }

    fn endpoint_out(&mut self, address: usb_device::endpoint::EndpointAddress) {
        if address != self.bulk_out.address() { return; }
        let mut packet = [0u8; BULK_MAX_PACKET_SIZE as usize];
        if let Ok(length) = self.bulk_out.read(&mut packet) {
            diagnostics::OUT_PACKETS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if length < CONTAINER_HEADER_LEN {
                diagnostics::OUT_INVALID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            }
            let Ok(header) = ContainerHeader::decode(&packet[..length]) else {
                diagnostics::OUT_INVALID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            };
            if header.kind != ContainerType::Command
                || header.length < CONTAINER_HEADER_LEN as u32
                || header.length as usize > length
            {
                diagnostics::OUT_INVALID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            }
            if self.transaction_active {
                // Do not let a new command overwrite a data packet or response
                // that is still owned by the bulk-IN endpoint.
                if self.pending_command_len == 0 && length <= self.pending_command.len() {
                    self.pending_command[..length].copy_from_slice(&packet[..length]);
                    self.pending_command_len = length;
                    diagnostics::COMMANDS_DEFERRED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                } else {
                    diagnostics::PENDING_COMMAND_OVERFLOW.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
            } else {
                self.transaction_active = true;
                diagnostics::COMMANDS_ACCEPTED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                self.handle_command(&packet[..length]);
            }
        }
    }

    fn endpoint_in_complete(&mut self, address: usb_device::endpoint::EndpointAddress) {
        if address != self.bulk_in.address() { return; }
        diagnostics::IN_COMPLETE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if self.tx_is_response {
            diagnostics::RESPONSE_IN_COMPLETED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        } else if self.pending_response.is_some() {
            diagnostics::DATA_IN_COMPLETED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        self.tx_in_flight = false;

        // A data container is followed by its MTP response. The transaction
        // remains active until that response has itself completed.
        if let Some((code, transaction_id)) = self.pending_response.take() {
            self.data_transfer_active = false;
            self.queue_response(code, transaction_id);
            return;
        }

        // Non-final chunks complete without an MTP response. Keep the
        // transaction locked until the main loop queues the next chunk.
        if self.data_transfer_active {
            return;
        }

        self.transaction_active = false;

        // Process at most one deferred command, and only after the preceding
        // transaction is completely off the wire.
        if self.pending_command_len != 0 {
            let length = self.pending_command_len;
            let mut packet = [0u8; BULK_MAX_PACKET_SIZE as usize];
            packet[..length].copy_from_slice(&self.pending_command[..length]);
            self.pending_command_len = 0;
            self.transaction_active = true;
            self.handle_command(&packet[..length]);
        }
    }

    fn poll(&mut self) {
        self.transmit_pending();
    }

    fn reset(&mut self) {
        diagnostics::USB_RESETS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        self.reset_protocol_state();
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


/// Validated root-level file metadata received in an MTP SendObjectInfo data
/// container. Names are restricted to printable ASCII for now; this avoids
/// lossy conversion and path traversal on the embedded filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadObjectInfo {
    pub storage_id: u32,
    pub parent: u32,
    pub size: u32,
    pub name: [u8; 64],
    pub name_len: u8,
}

impl UploadObjectInfo {
    /// Parse the fixed fields and PTP UTF-16 string in an ObjectInfo dataset.
    /// This parser intentionally accepts only root-level regular files.
    pub fn decode(dataset: &[u8]) -> core::result::Result<Self, MtpError> {
        const FIXED_LEN: usize = 52;
        if dataset.len() < FIXED_LEN + 1 {
            return Err(MtpError::InvalidContainerLength);
        }
        let u16_at = |at: usize| u16::from_le_bytes([dataset[at], dataset[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([
            dataset[at], dataset[at + 1], dataset[at + 2], dataset[at + 3],
        ]);

        let storage_id = u32_at(0);
        let format = u16_at(4);
        let protection = u16_at(6);
        let size = u32_at(8);
        let parent = u32_at(38);
        let association_type = u16_at(42);
        let count = dataset[FIXED_LEN] as usize;
        if storage_id != PLACEHOLDER_STORAGE_ID
            || format == 0x3001
            || protection != 0
            || association_type != 0
            || (parent != 0 && parent != 0xffff_ffff)
            || count < 2
        {
            return Err(MtpError::InvalidContainerLength);
        }

        let string_bytes = count.checked_mul(2).ok_or(MtpError::InvalidContainerLength)?;
        let end = FIXED_LEN + 1 + string_bytes;
        if end > dataset.len() {
            return Err(MtpError::InvalidContainerLength);
        }
        let mut name = [0u8; 64];
        let mut name_len = 0usize;
        for index in 0..count - 1 {
            let at = FIXED_LEN + 1 + index * 2;
            let unit = u16::from_le_bytes([dataset[at], dataset[at + 1]]);
            if !(0x20..=0x7e).contains(&unit) {
                return Err(MtpError::InvalidContainerLength);
            }
            let byte = unit as u8;
            if matches!(byte, b'/' | b'\\' | b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|') {
                return Err(MtpError::InvalidContainerLength);
            }
            if name_len == name.len() {
                return Err(MtpError::InvalidContainerLength);
            }
            name[name_len] = byte;
            name_len += 1;
        }
        let terminator_at = FIXED_LEN + 1 + (count - 1) * 2;
        if u16::from_le_bytes([dataset[terminator_at], dataset[terminator_at + 1]]) != 0
            || name_len == 0
            || &name[..name_len] == b"."
            || &name[..name_len] == b".."
        {
            return Err(MtpError::InvalidContainerLength);
        }

        Ok(Self { storage_id, parent, size, name, name_len: name_len as u8 })
    }
}

#[cfg(test)]
mod upload_info_tests {
    use super::*;

    #[test]
    fn decodes_root_file_object_info() {
        let mut data = [0u8; 52 + 1 + 12];
        data[0..4].copy_from_slice(&PLACEHOLDER_STORAGE_ID.to_le_bytes());
        data[4..6].copy_from_slice(&0x3000u16.to_le_bytes());
        data[8..12].copy_from_slice(&1234u32.to_le_bytes());
        data[38..42].copy_from_slice(&0u32.to_le_bytes());
        data[52] = 6;
        for (index, unit) in [b't' as u16, b'e' as u16, b's' as u16, b't' as u16, b'.' as u16].iter().enumerate() {
            let at = 53 + index * 2;
            data[at..at + 2].copy_from_slice(&unit.to_le_bytes());
        }
        let parsed = UploadObjectInfo::decode(&data).unwrap();
        assert_eq!(parsed.size, 1234);
        assert_eq!(&parsed.name[..parsed.name_len as usize], b"test.");
    }

    #[test]
    fn rejects_path_separators() {
        let mut data = [0u8; 52 + 1 + 8];
        data[0..4].copy_from_slice(&PLACEHOLDER_STORAGE_ID.to_le_bytes());
        data[4..6].copy_from_slice(&0x3000u16.to_le_bytes());
        data[52] = 4;
        for (index, unit) in [b'a' as u16, b'/' as u16, b'b' as u16].iter().enumerate() {
            let at = 53 + index * 2;
            data[at..at + 2].copy_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(UploadObjectInfo::decode(&data), Err(MtpError::InvalidContainerLength));
    }
}
