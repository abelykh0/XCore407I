#![no_std]

//! USB 2.0 High-Speed Mass Storage firmware for XCore407I.
//!
//! The USB class is deliberately separated from the storage implementation.
//! The MSC transport only sees 512-byte logical blocks; it does not know
//! whether those blocks are backed by LittleFS, NAND, or a test device.

pub mod block;
pub mod board;
pub mod bus;
pub mod fat16;
pub mod image;
pub mod littlefs_image;
pub mod msc;
pub mod scsi;
pub mod usb_hs;
