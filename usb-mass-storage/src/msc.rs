//! USB Mass Storage Class, Bulk-Only Transport (BOT), SCSI transparent
//! command set. One 512-byte sector is staged at a time (bounded RAM use).

use crate::block::{BlockDevice, BlockError, SECTOR_SIZE};
use crate::scsi;
use usb_device::bus::{InterfaceNumber, UsbBus, UsbBusAllocator};
use usb_device::class::{ControlIn, ControlOut, UsbClass};
use usb_device::control::{Recipient, RequestType};
use usb_device::descriptor::DescriptorWriter;
use usb_device::endpoint::{EndpointAddress, EndpointIn, EndpointOut, EndpointType};
use usb_device::Result;

const CBW_LEN: usize = 31;
const CSW_LEN: usize = 13;
const CBW_SIGNATURE: u32 = 0x4342_5355;
const CSW_SIGNATURE: u32 = 0x5342_5355;
const BULK_IN: u8 = 0x81;
const BULK_OUT: u8 = 0x02;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase { Idle, DataIn, DataOut, StatusIn }

/// SCSI transparent-command-set MSC device using BOT.
pub struct MassStorage<'a, B: UsbBus, D: BlockDevice> {
    interface: InterfaceNumber,
    ep_in: EndpointIn<'a, B>,
    ep_out: EndpointOut<'a, B>,
    storage: D,
    phase: Phase,
    tag: u32,
    residue: u32,
    sectors_left: u16,
    next_lba: u32,
    failed: bool,
    sense_key: u8,
    sense_asc: u8,
    sector: [u8; SECTOR_SIZE],
}

impl<'a, B: UsbBus, D: BlockDevice> MassStorage<'a, B, D> {
    pub fn new(allocator: &'a UsbBusAllocator<B>, storage: D) -> Self {
        let interface = allocator.interface();
        let ep_in = allocator.alloc(
            Some(EndpointAddress::from(BULK_IN)), EndpointType::Bulk, 512, 0
        ).expect("MSC bulk IN endpoint allocation");
        let ep_out = allocator.alloc(
            Some(EndpointAddress::from(BULK_OUT)), EndpointType::Bulk, 512, 0
        ).expect("MSC bulk OUT endpoint allocation");
        Self {
            interface, ep_in, ep_out, storage, phase: Phase::Idle,
            tag: 0, residue: 0, sectors_left: 0, next_lba: 0,
            failed: false, sense_key: 0, sense_asc: 0,
            sector: [0; SECTOR_SIZE],
        }
    }

    pub fn storage_mut(&mut self) -> &mut D { &mut self.storage }

    fn set_sense(&mut self, key: u8, asc: u8) {
        self.sense_key = key;
        self.sense_asc = asc;
    }

    fn fail(&mut self, key: u8, asc: u8) {
        self.failed = true;
        self.set_sense(key, asc);
        self.send_csw();
    }

    fn send_csw(&mut self) {
        let mut csw = [0u8; CSW_LEN];
        csw[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
        csw[4..8].copy_from_slice(&self.tag.to_le_bytes());
        csw[8..12].copy_from_slice(&self.residue.to_le_bytes());
        csw[12] = if self.failed { 1 } else { 0 };
        if self.ep_in.write(&csw).is_ok() {
            self.phase = Phase::StatusIn;
        }
    }

    fn send_bytes(&mut self, data: &[u8]) {
        let n = data.len().min(self.residue as usize);
        match self.ep_in.write(&data[..n]) {
            Ok(written) => {
                self.residue = self.residue.saturating_sub(written as u32);
                self.phase = Phase::DataIn;
            }
            Err(_) => self.fail(0x04, 0x44), // hardware/internal target failure
        }
    }

    fn send_next_sector(&mut self) {
        if self.sectors_left == 0 {
            self.send_csw();
            return;
        }
        match self.storage.read_sector(self.next_lba, &mut self.sector) {
            Ok(()) => {
                if self.ep_in.write(&self.sector).is_err() {
                    self.fail(0x04, 0x44);
                    return;
                }
                self.next_lba = self.next_lba.wrapping_add(1);
                self.sectors_left -= 1;
                self.residue = self.residue.saturating_sub(SECTOR_SIZE as u32);
                self.phase = Phase::DataIn;
            }
            Err(_) => self.fail(0x03, 0x11), // unrecovered read error
        }
    }

    fn process_cbw(&mut self, cbw: &[u8]) {
        if cbw.len() != CBW_LEN || u32::from_le_bytes(cbw[0..4].try_into().unwrap()) != CBW_SIGNATURE {
            self.phase = Phase::Idle;
            self.ep_in.stall();
            self.ep_out.stall();
            return;
        }

        self.tag = u32::from_le_bytes(cbw[4..8].try_into().unwrap());
        self.residue = u32::from_le_bytes(cbw[8..12].try_into().unwrap());
        let flags = cbw[12];
        let lun = cbw[13];
        let cdb_len = (cbw[14] & 0x1f) as usize;
        let cdb = &cbw[15..15 + cdb_len.min(16)];
        self.failed = false;
        self.sectors_left = 0;

        if lun != 0 || cdb.is_empty() {
            self.fail(0x05, 0x25);
            return;
        }

        match cdb[0] {
            0x00 => self.send_csw(), // TEST UNIT READY
            0x03 => { // REQUEST SENSE
                let mut d = [0u8; 18];
                d[0] = 0x70; d[2] = self.sense_key; d[7] = 10; d[12] = self.sense_asc;
                self.send_bytes(&d);
            }
            0x12 => { // INQUIRY
                let mut d = [0u8; 36];
                d[0] = 0; d[1] = 0x80; d[2] = 5; d[3] = 2; d[4] = 31;
                d[8..16].copy_from_slice(b"XCORE407");
                d[16..32].copy_from_slice(b"USB NAND DISK   ");
                d[32..36].copy_from_slice(b"1.0 ");
                self.send_bytes(&d);
            }
            0x1a => { // MODE SENSE(6): caching page omitted; writable medium
                self.send_bytes(&[3, 0, 0, 0]);
            }
            0x1b | 0x1e => self.send_csw(), // START STOP / PREVENT ALLOW
            0x25 => { // READ CAPACITY(10)
                let count = self.storage.sector_count();
                if count == 0 { self.fail(0x02, 0x3a); return; }
                let mut d = [0u8; 8];
                d[0..4].copy_from_slice(&(count - 1).to_be_bytes());
                d[4..8].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
                self.send_bytes(&d);
            }
            0x28 | 0x2a => { // READ(10) / WRITE(10)
                if cdb.len() < 10 { self.fail(0x05, 0x24); return; }
                let lba = u32::from_be_bytes(cdb[2..6].try_into().unwrap());
                let count = u16::from_be_bytes(cdb[7..9].try_into().unwrap());
                if lba.checked_add(count as u32).map_or(true, |end| end > self.storage.sector_count()) {
                    self.fail(0x05, 0x21);
                    return;
                }
                if self.residue != (count as u32 * SECTOR_SIZE as u32) {
                    // BOT permits host transfer lengths that differ from CDB;
                    // reject mismatches rather than silently desynchronize.
                    self.fail(0x05, 0x24);
                    return;
                }
                self.next_lba = lba;
                self.sectors_left = count;
                if count == 0 { self.send_csw(); }
                else if cdb[0] == 0x28 {
                    if flags & 0x80 == 0 { self.fail(0x05, 0x24); } else { self.send_next_sector(); }
                } else if flags & 0x80 != 0 {
                    self.fail(0x05, 0x24);
                } else {
                    self.phase = Phase::DataOut;
                }
            }
            0x35 => { // SYNCHRONIZE CACHE(10)
                match self.storage.sync() {
                    Ok(()) => self.send_csw(),
                    Err(_) => self.fail(0x04, 0x44),
                }
            }
            _ => self.fail(0x05, 0x20), // invalid/unsupported command
        }
    }

    fn receive_out(&mut self) {
        let mut packet = [0u8; SECTOR_SIZE];
        let Ok(n) = self.ep_out.read(&mut packet) else { return; };
        match self.phase {
            Phase::Idle => self.process_cbw(&packet[..n]),
            Phase::DataOut => {
                if n != SECTOR_SIZE {
                    self.fail(0x05, 0x24);
                    return;
                }
                match self.storage.write_sector(self.next_lba, &packet) {
                    Ok(()) => {
                        self.next_lba = self.next_lba.wrapping_add(1);
                        self.sectors_left -= 1;
                        self.residue = self.residue.saturating_sub(SECTOR_SIZE as u32);
                        if self.sectors_left == 0 { self.send_csw(); }
                    }
                    Err(BlockError::ReadOnly) => self.fail(0x07, 0x27),
                    Err(_) => self.fail(0x03, 0x0c),
                }
            }
            _ => {}
        }
    }
}

impl<B: UsbBus, D: BlockDevice> UsbClass<B> for MassStorage<'_, B, D> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(self.interface, 0x08, 0x06, 0x50)?;
        writer.endpoint(&self.ep_out)?;
        writer.endpoint(&self.ep_in)?;
        Ok(())
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let r = *xfer.request();
        if r.request_type == RequestType::Class
            && r.recipient == Recipient::Interface
            && r.index as u8 == u8::from(self.interface)
            && r.request == 0xfe
        {
            let _ = xfer.accept_with(&[0]); // only LUN 0
        }
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let r = *xfer.request();
        if r.request_type == RequestType::Class
            && r.recipient == Recipient::Interface
            && r.index as u8 == u8::from(self.interface)
            && r.request == 0xff
        {
            self.phase = Phase::Idle;
            self.sectors_left = 0;
            self.residue = 0;
            self.failed = false;
            self.ep_in.unstall();
            self.ep_out.unstall();
            let _ = xfer.accept();
        }
    }

    fn endpoint_out(&mut self, address: EndpointAddress) {
        if address == self.ep_out.address() { self.receive_out(); }
    }

    fn endpoint_in_complete(&mut self, address: EndpointAddress) {
        if address != self.ep_in.address() { return; }
        match self.phase {
            Phase::DataIn => {
                if self.sectors_left != 0 { self.send_next_sector(); }
                else { self.send_csw(); }
            }
            Phase::StatusIn => self.phase = Phase::Idle,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.sectors_left = 0;
        self.residue = 0;
        self.failed = false;
        self.ep_in.unstall();
        self.ep_out.unstall();
    }
}
