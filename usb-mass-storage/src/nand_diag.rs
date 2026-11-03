//! Destructive full-media test for the XCore407I Samsung K9F1G08U0E NAND.
//!
//! Factory-marked bad blocks are never erased or programmed, preserving their
//! markers. Every factory-good block is erased, checked page-by-page, then all
//! 64 pages are programmed with a deterministic data pattern and read-verified.
//! FSMC ECC is deliberately disabled: this qualifies raw NAND I/O only.

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
fn data_write(value: u8) { unsafe { write_volatile(DATA, value) } }
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

fn read_page(page: u32, out: &mut [u8; PAGE_BYTES]) -> bool {
    command(0x00);
    address(0);
    address(0);
    address(page as u8);
    address((page >> 8) as u8);
    command(0x30);
    if !wait_ready_rb() { return false; }
    for byte in out.iter_mut() { *byte = data_read(); }
    true
}

fn read_marker(page: u32) -> Option<u8> {
    command(0x00);
    address(PAGE_BYTES as u8);
    address((PAGE_BYTES >> 8) as u8);
    address(page as u8);
    address((page >> 8) as u8);
    command(0x30);
    if !wait_ready_rb() { return None; }
    Some(data_read())
}

fn erase_block(block: u32) -> bool {
    let page = block * PAGES_PER_BLOCK;
    command(0x60);
    address(page as u8);
    address((page >> 8) as u8);
    command(0xD0);
    if !wait_ready_rb() { return false; }
    command(0x70);
    data_read() & 0x01 == 0
}

fn program_page(page: u32, block: u32, page_in_block: u32) -> bool {
    command(0x80);
    address(0);
    address(0);
    address(page as u8);
    address((page >> 8) as u8);

    // tADL >= 100 ns between final address and first data cycle.
    cortex_m::asm::delay(1_000);

    for i in 0..PAGE_BYTES {
        data_write(pattern_byte(block, page_in_block, i));
    }

    command(0x10);
    if !wait_ready_rb() { return false; }
    command(0x70);
    data_read() & 0x01 == 0
}

#[inline]
fn pattern_byte(block: u32, page: u32, offset: usize) -> u8 {
    (block as u8)
        .wrapping_mul(29)
        .wrapping_add((page as u8).wrapping_mul(71))
        .wrapping_add((offset as u8).wrapping_mul(37))
        ^ ((offset >> 8) as u8).rotate_left((page & 7) as u32)
        ^ 0xA5
}

fn verify_erased(block: u32, page_buf: &mut [u8; PAGE_BYTES]) -> Result<(), (u32, u16)> {
    for p in 0..PAGES_PER_BLOCK {
        if !read_page(block * PAGES_PER_BLOCK + p, page_buf) {
            return Err((p, 0xFE)); // read timeout
        }
        if let Some(offset) = page_buf.iter().position(|&v| v != 0xFF) {
            return Err((p, offset as u16));
        }
    }
    Ok(())
}

fn verify_programmed(
    block: u32,
    page: u32,
    page_buf: &mut [u8; PAGE_BYTES],
) -> Result<(), (u32, u16, u8, u8)> {
    let absolute_page = block * PAGES_PER_BLOCK + page;
    if !read_page(absolute_page, page_buf) {
        return Err((page, 0xFFFF, 0x00, 0xFE));
    }
    for (offset, actual) in page_buf.iter().copied().enumerate() {
        let expected = pattern_byte(block, page, offset);
        if actual != expected {
            return Err((page, offset as u16, expected, actual));
        }
    }
    Ok(())
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
        // NAND bank 2, 8-bit bus, FSMC ECC disabled, NWAIT enabled.
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

fn append_hex(out: &mut [u8; 192], len: &mut usize, value: u32, digits: usize) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as usize;
        append(out, len, &HEX[nibble..nibble + 1]);
    }
}

/// Erases and fully tests every factory-good block. Bad-block marker pages
/// are inspected first and bad blocks are skipped. The report is served as
/// README.txt over MTP; USB enumeration starts only after the test completes.
pub fn create_report() -> ([u8; 192], usize) {
    let mut report = [0u8; 192];
    let mut len = 0;
    append(&mut report, &mut len, b"XCore NAND full test\r\n");
    configure_fsmc();
    command(0xFF);
    if !wait_ready_status() {
        append(&mut report, &mut len, b"ERROR: reset timeout\r\n");
        return (report, len);
    }

    command(0x90);
    address(0);
    let mut id = [0u8; 5];
    for byte in &mut id { *byte = data_read(); }
    append(&mut report, &mut len, b"ID: ");
    for (i, byte) in id.iter().enumerate() {
        if i != 0 { append(&mut report, &mut len, b" "); }
        append_hex(&mut report, &mut len, *byte as u32, 2);
    }
    append(&mut report, &mut len, b"\r\n");
    if id[0] != 0xEC || id[1] != 0xF1 {
        append(&mut report, &mut len, b"ERROR: unexpected NAND ID\r\n");
        return (report, len);
    }

    // Preserve factory bad-block markers by never erasing a marked block.
    let mut good = 0u16;
    let mut bad = 0u16;
    for block in 0..BLOCKS {
        let first = read_marker(block * PAGES_PER_BLOCK);
        let second = read_marker(block * PAGES_PER_BLOCK + 1);
        match (first, second) {
            (Some(0xFF), Some(0xFF)) => good += 1,
            (Some(_), Some(_)) => bad += 1,
            _ => {
                append(&mut report, &mut len, b"ERROR: marker scan timeout\r\n");
                return (report, len);
            }
        }
    }
    append(&mut report, &mut len, b"Good: ");
    append_hex(&mut report, &mut len, good as u32, 4);
    append(&mut report, &mut len, b" bad: ");
    append_hex(&mut report, &mut len, bad as u32, 4);
    append(&mut report, &mut len, b"\r\n");

    let mut erased = 0u16;
    let mut programmed = 0u16;
    let mut failed_block = None;
    let mut failed_phase = 0u8;
    let mut failed_page = 0u8;
    let mut failed_offset = 0u16;
    let mut expected = 0u8;
    let mut actual = 0u8;
    let mut page_buf = [0u8; PAGE_BYTES];

    for block in 0..BLOCKS {
        let first = match read_marker(block * PAGES_PER_BLOCK) {
            Some(v) => v,
            None => { failed_block = Some(block); failed_phase = 1; break; }
        };
        let second = match read_marker(block * PAGES_PER_BLOCK + 1) {
            Some(v) => v,
            None => { failed_block = Some(block); failed_phase = 1; break; }
        };
        if first != 0xFF || second != 0xFF { continue; }

        if !erase_block(block) {
            failed_block = Some(block); failed_phase = 2; break;
        }
        if let Err((page, offset)) = verify_erased(block, &mut page_buf) {
            failed_block = Some(block); failed_phase = 3;
            failed_page = page as u8; failed_offset = offset as u16;
            break;
        }
        erased += 1;

        for page in 0..PAGES_PER_BLOCK {
            if !program_page(block * PAGES_PER_BLOCK + page, block, page) {
                failed_block = Some(block); failed_phase = 4;
                failed_page = page as u8; break;
            }
            if let Err((bad_page, offset, exp, got)) =
                verify_programmed(block, page, &mut page_buf)
            {
                failed_block = Some(block); failed_phase = 5;
                failed_page = bad_page as u8; failed_offset = offset;
                expected = exp; actual = got; break;
            }
        }
        if failed_block.is_some() { break; }
        programmed += 1;
    }

    append(&mut report, &mut len, b"Erase+verify: ");
    append_hex(&mut report, &mut len, erased as u32, 4);
    append(&mut report, &mut len, b"\r\nWrite+verify: ");
    append_hex(&mut report, &mut len, programmed as u32, 4);
    append(&mut report, &mut len, b"\r\n64 pages x 2048 B/block\r\n");
    append(&mut report, &mut len, b"ECC: OFF (raw)\r\n");
    if let Some(block) = failed_block {
        append(&mut report, &mut len, b"FAIL block 0x");
        append_hex(&mut report, &mut len, block, 3);
        append(&mut report, &mut len, b" phase ");
        append_hex(&mut report, &mut len, failed_phase as u32, 1);
        append(&mut report, &mut len, b" page ");
        append_hex(&mut report, &mut len, failed_page as u32, 2);
        append(&mut report, &mut len, b" off ");
        append_hex(&mut report, &mut len, failed_offset as u32, 4);
        append(&mut report, &mut len, b" exp ");
        append_hex(&mut report, &mut len, expected as u32, 2);
        append(&mut report, &mut len, b" got ");
        append_hex(&mut report, &mut len, actual as u32, 2);
        append(&mut report, &mut len, b"\r\n");
    } else {
        append(&mut report, &mut len, b"PASS: all good blocks\r\n");
    }
    append(&mut report, &mut len, b"Factory-bad blocks skipped.\r\n");
    (report, len)
}
