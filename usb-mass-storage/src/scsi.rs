//! SCSI transparent command set constants used by the MSC BOT transport.

pub const TEST_UNIT_READY: u8 = 0x00;
pub const REQUEST_SENSE: u8 = 0x03;
pub const INQUIRY: u8 = 0x12;
pub const MODE_SENSE_6: u8 = 0x1a;
pub const START_STOP_UNIT: u8 = 0x1b;
pub const PREVENT_ALLOW_MEDIUM_REMOVAL: u8 = 0x1e;
pub const READ_CAPACITY_10: u8 = 0x25;
pub const READ_10: u8 = 0x28;
pub const WRITE_10: u8 = 0x2a;
pub const SYNCHRONIZE_CACHE_10: u8 = 0x35;

pub const SENSE_NO_SENSE: u8 = 0x00;
pub const SENSE_NOT_READY: u8 = 0x02;
pub const SENSE_MEDIUM_ERROR: u8 = 0x03;
pub const SENSE_HARDWARE_ERROR: u8 = 0x04;
pub const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
pub const SENSE_DATA_PROTECT: u8 = 0x07;
