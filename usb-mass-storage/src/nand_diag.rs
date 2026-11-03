//! Non-destructive K9F1G08U0E factory-marker scan for the MTP README.
//!
//! This reports the NAND ID and counts factory-marked bad blocks. It does not
//! erase/program NAND and does not perform ECC correction or data validation.

use core::ptr::{read_volatile, write_volatile};

const FSMC_BASE: usize = 0xA000_0000;
const NAND_BASE: usize = 0x7000_0000;
const GPIOD_IDR: *const u32 = 0x4002_0C10 as *const u32;
const RB_MASK: u32 = 1 << 6;
const CMD: *mut u8 = (NAND_BASE | (1 << 17)) as *mut u8;
const ADDR: *mut u8 = (NAND_BASE | (1 << 16)) as *mut u8;
const DATA: *mut u8 = NAND_BASE as *mut u8;
const TIMEOUT: u32 = 20_000_000;

const PAGE_BYTES: usize = 2048;
const PAGES_PER_BLOCK: u32 = 64;
const BLOCKS: u32 = 1024;

fn command(value: u8) { unsafe { write_volatile(CMD, value) } }
fn address(value: u8) { unsafe { write_volatile(ADDR, value) } }
fn data_read() -> u8 { unsafe { read_volatile(DATA) } }

fn wait_ready_status() -> bool {
    for _ in 0..TIMEOUT {
        command(0x70);
        if data_read() & 0x40 != 0 { return true; }
        core::hint::spin_loop();
    }
    false
}

fn wait_ready_rb() -> bool {
    let mut busy_seen = false;
    for _ in 0..TIMEOUT {
        let ready = unsafe { read_volatile(GPIOD_IDR) } & RB_MASK != 0;
        if !ready {
            busy_seen = true;
        } else if busy_seen {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn read_page_byte(page: u32, column: u16) -> Option<u8> {
    command(0x00);
    address(column as u8);
    address((column >> 8) as u8);
    address(page as u8);
    address((page >> 8) as u8);
    command(0x30);
    if !wait_ready_rb() { return None; }
    Some(data_read())
}

fn configure_fsmc() {
    unsafe {
        let rcc_ahb3enr = 0x4002_3838 as *mut u32;
        write_volatile(rcc_ahb3enr, read_volatile(rcc_ahb3enr) | 1);
        let pcr2 = (FSMC_BASE + 0x60) as *mut u32;
        let timing = 4 | (8 << 8) | (4 << 16) | (2 << 24);
        write_volatile(pcr2, 0);
        write_volatile((FSMC_BASE + 0x68) as *mut u32, timing);
        write_volatile((FSMC_BASE + 0x6C) as *mut u32, timing);
        let control = (1 << 1) | (1 << 2) | (1 << 3) | (2 << 9) | (2 << 13);
        write_volatile(pcr2, control);
        let _ = read_volatile(pcr2);
    }
}

fn append(out: &mut [u8; 192], len: &mut usize, bytes: &[u8]) {
    let count = bytes.len().min(out.len() - *len);
    out[*len..*len + count].copy_from_slice(&bytes[..count]);
    *len += count;
}

fn append_hex_byte(out: &mut [u8; 192], len: &mut usize, value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    append(out, len, &[HEX[(value >> 4) as usize], HEX[(value & 0x0F) as usize]]);
}

fn append_hex_u16(out: &mut [u8; 192], len: &mut usize, value: u16) {
    append_hex_byte(out, len, (value >> 8) as u8);
    append_hex_byte(out, len, value as u8);
}

/// Run a read-only NAND identification and factory bad-block scan.
/// Returns a text report suitable for the MTP README object.
pub fn create_report() -> ([u8; 192], usize) {
    let mut report = [0u8; 192];
    let mut len = 0;
    append(&mut report, &mut len, b"XCore407I NAND diagnostic\r\n");

    configure_fsmc();
    command(0xFF);
    if !wait_ready_status() {
        append(&mut report, &mut len, b"ERROR: NAND reset timeout\r\n");
        return (report, len);
    }

    command(0x90);
    address(0);
    let mut id = [0u8; 5];
    for byte in &mut id { *byte = data_read(); }
    append(&mut report, &mut len, b"ID: ");
    for (index, byte) in id.iter().enumerate() {
        if index != 0 { append(&mut report, &mut len, b" "); }
        append_hex_byte(&mut report, &mut len, *byte);
    }
    append(&mut report, &mut len, b"\r\n");

    if id[0] != 0xEC || id[1] != 0xF1 {
        append(&mut report, &mut len, b"ERROR: unexpected NAND ID\r\n");
        return (report, len);
    }

    let mut good = 0u16;
    let mut bad = 0u16;
    for block in 0..BLOCKS {
        let first = block * PAGES_PER_BLOCK;
        let marker0 = read_page_byte(first, PAGE_BYTES as u16);
        let marker1 = read_page_byte(first + 1, PAGE_BYTES as u16);
        match (marker0, marker1) {
            (Some(a), Some(b)) if a == 0xFF && b == 0xFF => good += 1,
            (Some(_), Some(_)) => bad += 1,
            _ => {
                append(&mut report, &mut len, b"ERROR: marker read timeout\r\n");
                return (report, len);
            }
        }
    }

    append(&mut report, &mut len, b"Factory-good blocks: 0x");
    append_hex_u16(&mut report, &mut len, good);
    append(&mut report, &mut len, b" (");
    // Decimal conversion without formatting or allocation.
    let mut digits = [0u8; 5];
    let mut value = good as u32;
    let mut used = 0;
    loop {
        digits[used] = b'0' + (value % 10) as u8;
        used += 1;
        value /= 10;
        if value == 0 { break; }
    }
    while used != 0 {
        used -= 1;
        append(&mut report, &mut len, &digits[used..used + 1]);
    }
    append(&mut report, &mut len, b")\r\nFactory-bad blocks: 0x");
    append_hex_u16(&mut report, &mut len, bad);
    append(&mut report, &mut len, b" (");
    let mut value = bad as u32;
    let mut used = 0;
    loop {
        digits[used] = b'0' + (value % 10) as u8;
        used += 1;
        value /= 10;
        if value == 0 { break; }
    }
    while used != 0 {
        used -= 1;
        append(&mut report, &mut len, &digits[used..used + 1]);
    }
    append(&mut report, &mut len, b")\r\n");
    append(&mut report, &mut len, b"ECC: not applied; raw marker scan only.\r\n");
    append(&mut report, &mut len, b"Erase/program: none.\r\n");
    (report, len)
}
