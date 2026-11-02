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
    // register and destroys the pending page-data read stream.
    //
    // R/B is normally high while idle. Waiting only for high can therefore
    // return immediately, before the NAND has even entered its busy interval.
    // First observe the low-going busy edge, then wait for the high-going
    // ready edge. Page read (tR), program (tPROG), and erase (tBERS) all assert
    // R/B low, so this also prevents reading stale data from the previous
    // operation.
    let mut saw_busy = false;
    for _ in 0..TIMEOUT {
        let ready = unsafe { read_volatile(GPIOD_IDR) } & NAND_RB_MASK != 0;
        if !ready {
            saw_busy = true;
        } else if saw_busy {
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
    // Call only after wait_ready_rb() has observed R/B high.
    write_cmd(CMD_STATUS);
    Ok(read_data())
}

fn program_page(page: u32) -> Result<(), Error> {
    write_cmd(CMD_PROGRAM);
    row_address(page);

    // K9F1G08U0E specifies tADL >= 100 ns from the final address
    // WE rising edge to the first data WE rising edge. Do not rely on
    // the current FSMC timing configuration to provide that margin.
    cortex_m::asm::delay(1_000);

    for i in 0..PAGE_BYTES {
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
    row_address(page);
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

fn hex_digit(value: u8) -> u8 {
    b"0123456789ABCDEF"[(value & 0x0F) as usize]
}

fn show_hex_bytes(prefix: &[u8], bytes: &[u8]) {
    let mut line = [b' '; 32];
    let mut pos = 0;
    for &ch in prefix {
        if pos < line.len() { line[pos] = ch; pos += 1; }
    }
    for (index, &byte) in bytes.iter().enumerate() {
        if pos + 3 > line.len() { break; }
        if index != 0 { line[pos] = b' '; pos += 1; }
        line[pos] = hex_digit(byte >> 4);
        line[pos + 1] = hex_digit(byte);
        pos += 2;
    }
    show_usb_status(&line);
}

/// Capture independent program status and raw data/spare reads before verify.
/// Each call advances to a new diagnostic text row on the camera canvas.
fn show_program_diagnostics(page: u32) -> Result<(), Error> {
    // Read the status register immediately after program completion, before
    // issuing any other NAND command. Bit 7=1 means not write-protected;
    // bit 0=1 means program/erase failure.
    let status = read_status()?;
    let mut status_line = *b"STATUS: 00 WP:0 FAIL:0 ";
    status_line[8] = hex_digit(status >> 4);
    status_line[9] = hex_digit(status);
    status_line[14] = if status & 0x80 != 0 { b'1' } else { b'0' };
    status_line[21] = if status & 0x01 != 0 { b'1' } else { b'0' };
    show_usb_status(&status_line);

    // Reset the NAND after capturing program status. This separates the
    // subsequent array reads from any lingering status-register/output state.
    write_cmd(CMD_RESET);
    wait_ready_status()?;

    // Read short windows at two main-area columns and at the spare-area start.
    // Eight bytes fit on one 32-character status row without truncation.
    let mut data0 = [0u8; 8];
    read_page(page, 0, &mut data0)?;
    show_hex_bytes(b"COL000:", &data0);

    let mut data100 = [0u8; 8];
    read_page(page, 0x0100, &mut data100)?;
    show_hex_bytes(b"COL100:", &data100);

    let mut spare = [0u8; 8];
    read_page(page, PAGE_BYTES as u16, &mut spare)?;
    show_hex_bytes(b"SPARE:", &spare);
    Ok(())
}


fn show_register(label: &[u8], address: usize) {
    let value = unsafe { read_volatile(address as *const u32) };
    let mut line = [b' '; 32];
    let mut pos = 0;
    for &ch in label {
        if pos < line.len() { line[pos] = ch; pos += 1; }
    }
    if pos + 8 <= line.len() {
        for shift in (0..8).rev() {
            line[pos] = hex_digit((value >> (shift * 4)) as u8);
            pos += 1;
        }
    }
    show_usb_status(&line);
}

fn show_bus_configuration() {
    show_register(b"PCR2:", FSMC_BASE + 0x60);
    show_register(b"PMEM2:", FSMC_BASE + 0x68);
    show_register(b"PATT2:", FSMC_BASE + 0x6C);
    show_register(b"SR2:", FSMC_BASE + 0x64);
    show_register(b"GPIOD MOD:", 0x4002_0C00);
    show_register(b"GPIOD AFRL:", 0x4002_0C20);
    show_register(b"GPIOD AFRH:", 0x4002_0C24);
    show_register(b"GPIOE MOD:", 0x4002_1000);
    show_register(b"GPIOE AFRL:", 0x4002_1020);
    show_register(b"GPIOE AFRH:", 0x4002_1024);
    show_register(b"RCC AHB1ENR:", 0x4002_3830);
    show_register(b"RCC AHB3ENR:", 0x4002_3838);
    show_register(b"GPIOD IDR:", 0x4002_0C10);
    show_register(b"GPIOD ODR:", 0x4002_0C14);
    show_register(b"GPIOE IDR:", 0x4002_1010);
    show_register(b"GPIOE ODR:", 0x4002_1014);
    show_register(b"GPIOD OSPEED:", 0x4002_0C08);
    show_register(b"GPIOE OSPEED:", 0x4002_1008);
}

fn configure_fsmc() {
    unsafe {
        let rcc_ahb3enr = 0x4002_3838 as *mut u32;
        write_volatile(rcc_ahb3enr, read_volatile(rcc_ahb3enr) | 1);
        let pcr2 = (FSMC_BASE + 0x60) as *mut u32;
        write_volatile(pcr2, 0);
        let timing = 4 | (8 << 8) | (4 << 16) | (2 << 24);
        write_volatile((FSMC_BASE + 0x68) as *mut u32, timing);
        write_volatile((FSMC_BASE + 0x6C) as *mut u32, timing);
        let control = (1 << 2) | (1 << 3) | (2 << 9) | (2 << 13);
        write_volatile(pcr2, control);
        let _ = read_volatile(pcr2);
    }
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
    show_bus_configuration();

    write_cmd(CMD_RESET);
    if let Err(e) = wait_ready_status() {
        report_error(e);
        return;
    }

    let mut id = [0u8; 5];
    read_id(&mut id);

    // One compact status row: ID0 through ID4, in byte order.
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut id_line = *b"ID0..4: 00 00 00 00 00";
    for (index, value) in id.iter().copied().enumerate() {
        let offset = 8 + index * 3;
        id_line[offset] = HEX[(value >> 4) as usize];
        id_line[offset + 1] = HEX[(value & 0x0F) as usize];
    }
    show_usb_status(&id_line);

    if id[0] != 0xEC || id[1] != 0xF1 {
        report_error(Error::WrongId);
        return;
    }

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

    if let Err(e) = show_program_diagnostics(page) {
        report_error(e);
        return;
    }

    if let Err(e) = verify_page(page) {
        report_error(e);
        return;
    }
    show_usb_status(b"NAND: VERIFY OK");
    show_usb_status(b"NAND TEST: PASS");
}
