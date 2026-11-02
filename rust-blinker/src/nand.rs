//! Minimal raw NAND bring-up test for the XCore407I K9F1G08U0E.
//!
//! Uses FSMC NAND bank 2 (PD7/NCE2), with CLE=A17 and ALE=A16.
//! This is a destructive manufacturing-style test: it erases one good block.
//! It deliberately does not touch block 0 and checks both factory bad-block
//! markers before selecting a test block.

use crate::camera::show_usb_status;
use core::ptr::{read_volatile, write_volatile};

const FSMC_BASE: usize = 0xA000_0000;
const NAND_BASE: usize = 0x7000_0000;
// PD6 is connected to NAND R/B and remains readable through GPIOx_IDR even
// while the pin is configured for FSMC AF12.
const GPIOD_IDR: *const u32 = 0x4002_0C10 as *const u32;
const NAND_RB_MASK: u32 = 1 << 6;
const CMD_ADDR: *mut u8 = (NAND_BASE | (1 << 17)) as *mut u8; // CLE -> A17
const ADDR_ADDR: *mut u8 = (NAND_BASE | (1 << 16)) as *mut u8; // ALE -> A16
const DATA_ADDR: *mut u8 = NAND_BASE as *mut u8;

const PAGE_BYTES: usize = 2048;
const SPARE_BYTES: usize = 64;
const PAGES_PER_BLOCK: u32 = 64;
const BLOCK_COUNT: u32 = 1024;
const TIMEOUT: u32 = 20_000_000;

const CMD_READ0: u8 = 0x00;
const CMD_READSTART: u8 = 0x30;
const CMD_READID: u8 = 0x90;
const CMD_STATUS: u8 = 0x70;
const CMD_PROGRAM: u8 = 0x80;
const CMD_PROGRAM_CONFIRM: u8 = 0x10;
const CMD_ERASE: u8 = 0x60;
const CMD_ERASE_CONFIRM: u8 = 0xD0;
const CMD_RESET: u8 = 0xFF;

#[derive(Clone, Copy)]
enum Error {
    Timeout,
    WrongId,
    NoGoodBlock,
    EraseFailed,
    ProgramFailed,
    ReadFailed,
    VerifyFailed(u16, u8, u8),
}

fn write_cmd(value: u8) {
    unsafe { write_volatile(CMD_ADDR, value) };
}

fn write_addr(value: u8) {
    unsafe { write_volatile(ADDR_ADDR, value) };
}

fn write_data(value: u8) {
    unsafe { write_volatile(DATA_ADDR, value) };
}

fn read_data() -> u8 {
    unsafe { read_volatile(DATA_ADDR) }
}

fn wait_ready_rb() -> Result<(), Error> {
    // Do not issue 0x70 here: that changes the NAND's output mux to the status
    // register and destroys the pending page-data read stream. Poll R/B.
    for _ in 0..TIMEOUT {
        if unsafe { read_volatile(GPIOD_IDR) } & NAND_RB_MASK != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(Error::Timeout)
}

fn wait_ready_status() -> Result<(), Error> {
    // Safe after RESET: there is no page-data stream to preserve.
    for _ in 0..TIMEOUT {
        write_cmd(CMD_STATUS);
        if read_data() & 0x40 != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(Error::Timeout)
}

fn row_address(page: u32) {
    write_addr(0); // column low
    write_addr(0); // column high
    write_addr(page as u8);
    write_addr((page >> 8) as u8);
}

fn read_page(page: u32, column: u16, out: &mut [u8]) -> Result<(), Error> {
    write_cmd(CMD_READ0);
    write_addr(column as u8);
    write_addr((column >> 8) as u8);
    write_addr(page as u8);
    write_addr((page >> 8) as u8);
    write_cmd(CMD_READSTART);
    wait_ready_rb()?;
    for byte in out {
        *byte = read_data();
    }
    Ok(())
}

fn read_id(id: &mut [u8; 5]) {
    write_cmd(CMD_READID);
    write_addr(0);
    for byte in id {
        *byte = read_data();
    }
}

fn block_is_good(block: u32) -> Result<bool, Error> {
    let mut marker = [0u8; 1];
    for page_in_block in 0..2 {
        let page = block * PAGES_PER_BLOCK + page_in_block;
        read_page(page, PAGE_BYTES as u16, &mut marker)?;
        if marker[0] != 0xFF {
            return Ok(false);
        }
    }
    Ok(true)
}

fn erase_block(block: u32) -> Result<(), Error> {
    let page = block * PAGES_PER_BLOCK;
    write_cmd(CMD_ERASE);
    write_addr(page as u8);
    write_addr((page >> 8) as u8);
    write_cmd(CMD_ERASE_CONFIRM);
    wait_ready_rb()?;
    if read_status()? & 0x01 != 0 {
        return Err(Error::EraseFailed);
    }
    Ok(())
}

fn read_status() -> Result<u8, Error> {
    // Call only after wait_ready() has observed R/B high. Read the status byte
    // directly; calling wait_ready() after 0x70 would issue another command.
    write_cmd(CMD_STATUS);
    Ok(read_data())
}

fn program_page(page: u32) -> Result<(), Error> {
    write_cmd(CMD_PROGRAM);
    row_address(page);
    for i in 0..PAGE_BYTES {
        // Deterministic nontrivial pattern, different in every byte position.
        write_data((i as u8).wrapping_mul(37) ^ ((i >> 3) as u8) ^ 0xA5);
    }
    write_cmd(CMD_PROGRAM_CONFIRM);
    wait_ready_rb()?;
    if read_status()? & 0x01 != 0 {
        return Err(Error::ProgramFailed);
    }
    Ok(())
}

fn verify_page(page: u32) -> Result<(), Error> {
    write_cmd(CMD_READ0);
    write_addr(0);
    write_addr(0);
    write_addr(page as u8);
    write_addr((page >> 8) as u8);
    write_addr((page >> 16) as u8);
    write_cmd(CMD_READSTART);
    wait_ready_rb()?;

    for i in 0..PAGE_BYTES {
        let actual = read_data();
        let expected = (i as u8).wrapping_mul(37) ^ ((i >> 3) as u8) ^ 0xA5;
        if actual != expected {
            return Err(Error::VerifyFailed(i as u16, expected, actual));
        }
    }
    Ok(())
}

fn configure_fsmc() {
    unsafe {
        // AHB3 FSMC clock.
        let rcc_ahb3enr = 0x4002_3838 as *mut u32;
        write_volatile(rcc_ahb3enr, read_volatile(rcc_ahb3enr) | 1);

        // Disable bank while programming timing and control registers.
        let pcr2 = (FSMC_BASE + 0x60) as *mut u32;
        write_volatile(pcr2, 0);

        // Conservative asynchronous NAND timings at 168 MHz HCLK.
        // PMEM/PATT fields: setup=4, wait=8, hold=4, Hi-Z=2 HCLK cycles.
        let timing = 4 | (8 << 8) | (4 << 16) | (2 << 24);
        write_volatile((FSMC_BASE + 0x68) as *mut u32, timing);
        write_volatile((FSMC_BASE + 0x6C) as *mut u32, timing);

        // NAND bank 2, 8-bit bus, CLE/ALE timing margins, no hardware ECC.
        // PBKEN=1, PTYP=1 (NAND), PWID=00 (8-bit), TCLR=2, TAR=2.
        let control = (1 << 2) | (1 << 3) | (2 << 9) | (2 << 13);
        write_volatile(pcr2, control);
        let _ = read_volatile(pcr2);
    }
}

fn emit_hex(prefix: &[u8], value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut line = [b' '; 32];
    let len = prefix.len().min(26);
    line[..len].copy_from_slice(&prefix[..len]);
    line[len] = HEX[(value >> 4) as usize];
    line[len + 1] = HEX[(value & 0x0F) as usize];
    show_usb_status(&line[..len + 2]);
}

fn report_error(error: Error) {
    match error {
        Error::Timeout => show_usb_status(b"NAND: TIMEOUT"),
        Error::WrongId => show_usb_status(b"NAND: UNEXPECTED ID"),
        Error::NoGoodBlock => show_usb_status(b"NAND: NO GOOD BLOCK"),
        Error::EraseFailed => show_usb_status(b"NAND: ERASE FAIL"),
        Error::ProgramFailed => show_usb_status(b"NAND: PROGRAM FAIL"),
        Error::ReadFailed => show_usb_status(b"NAND: READ FAIL"),
        Error::VerifyFailed(offset, expected, actual) => {
            show_usb_status(b"NAND: VERIFY FAIL");
            let mut line = [b' '; 24];
            line[..4].copy_from_slice(b"OFF ");
            let n = offset as usize;
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            line[4] = HEX[(n >> 12) & 0xF];
            line[5] = HEX[(n >> 8) & 0xF];
            line[6] = HEX[(n >> 4) & 0xF];
            line[7] = HEX[n & 0xF];
            line[8..11].copy_from_slice(b" EX");
            line[11] = HEX[(expected >> 4) as usize];
            line[12] = HEX[(expected & 0xF) as usize];
            line[13..16].copy_from_slice(b" AC");
            line[16] = HEX[(actual >> 4) as usize];
            line[17] = HEX[(actual & 0xF) as usize];
            show_usb_status(&line);
        }
    }
}

/// Initialize FSMC, identify the NAND, then erase/program/verify one good block.
pub fn run_test() {
    show_usb_status(b"NAND: FSMC INIT");
    configure_fsmc();

    // Reset the device and wait until it is ready.
    write_cmd(CMD_RESET);
    if let Err(e) = wait_ready_status() {
        report_error(e);
        return;
    }

    let mut id = [0u8; 5];
    read_id(&mut id);
    emit_hex(b"NAND ID0: ", id[0]);
    emit_hex(b"NAND ID1: ", id[1]);
    emit_hex(b"NAND ID2: ", id[2]);
    emit_hex(b"NAND ID3: ", id[3]);
    emit_hex(b"NAND ID4: ", id[4]);

    if id[0] != 0xEC || id[1] != 0xF1 {
        report_error(Error::WrongId);
        return;
    }

    // Samsung specifies 1,024 blocks, 64 pages/block, 2,048+64 bytes/page.
    // Never erase block 0; scan the remaining blocks for untouched factory
    // markers in the first two pages before selecting a test block.
    let mut selected = None;
    for block in 1..BLOCK_COUNT {
        match block_is_good(block) {
            Ok(true) => {
                selected = Some(block);
                break;
            }
            Ok(false) => {}
            Err(e) => {
                report_error(e);
                return;
            }
        }
    }

    let block = match selected {
        Some(value) => value,
        None => {
            report_error(Error::NoGoodBlock);
            return;
        }
    };

    show_usb_status(b"NAND: GOOD BLOCK FOUND");
    if let Err(e) = erase_block(block) {
        report_error(e);
        return;
    }
    show_usb_status(b"NAND: ERASE OK");

    let page = block * PAGES_PER_BLOCK;
    if let Err(e) = program_page(page) {
        report_error(e);
        return;
    }
    show_usb_status(b"NAND: PROGRAM OK");

    if let Err(e) = verify_page(page) {
        report_error(e);
        return;
    }
    show_usb_status(b"NAND: VERIFY OK");
    show_usb_status(b"NAND TEST: PASS");
}
