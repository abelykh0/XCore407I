//! Formatter for the fixed-size FAT16 volume stored inside a LittleFS file.
//!
//! This does not implement a second filesystem in firmware. It creates the
//! standard on-disk structures once; Windows subsequently owns the FAT volume
//! through MSC. The emulator can later mount the same image through FatFs.

use crate::block::{BlockError, SECTOR_SIZE};
use crate::image::ImageFile;

pub const DEFAULT_VOLUME_SECTORS: u32 = 65_536; // 32 MiB
const SECTORS_PER_CLUSTER: u8 = 4;
const RESERVED_SECTORS: u16 = 1;
const FAT_COUNT: u8 = 2;
const ROOT_ENTRIES: u16 = 512;
const ROOT_DIR_SECTORS: u32 = (ROOT_ENTRIES as u32 * 32).div_ceil(SECTOR_SIZE as u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fat16Geometry {
    pub total_sectors: u32,
    pub sectors_per_fat: u16,
    pub root_dir_sectors: u32,
    pub first_data_sector: u32,
    pub cluster_count: u32,
}

pub fn geometry(total_sectors: u32) -> Result<Fat16Geometry, BlockError> {
    if total_sectors < 4_096 {
        return Err(BlockError::OutOfRange);
    }
    let mut sectors_per_fat = 1u32;
    let root = ROOT_DIR_SECTORS;
    for _ in 0..8 {
        let first_data = RESERVED_SECTORS as u32 + FAT_COUNT as u32 * sectors_per_fat + root;
        if first_data >= total_sectors { return Err(BlockError::OutOfRange); }
        let clusters = (total_sectors - first_data) / SECTORS_PER_CLUSTER as u32;
        let needed = ((clusters + 2) * 2).div_ceil(SECTOR_SIZE as u32);
        if needed == sectors_per_fat { 
            if clusters < 4_085 || clusters >= 65_525 { return Err(BlockError::OutOfRange); }
            return Ok(Fat16Geometry {
                total_sectors, sectors_per_fat: needed as u16,
                root_dir_sectors: root, first_data_sector: first_data,
                cluster_count: clusters,
            });
        }
        sectors_per_fat = needed;
    }
    Err(BlockError::Io)
}

/// Initialize a FAT16 image. The image file must already be created in the
/// backing filesystem. This writes zeroes in bounded 4 KiB chunks, then writes
/// the boot sector and both FAT headers. Peak scratch RAM is 4 KiB.
pub fn format<F: ImageFile>(image: &mut F, total_sectors: u32) -> Result<Fat16Geometry, BlockError> {
    let g = geometry(total_sectors)?;
    // Clear old FAT metadata and file data. LittleFS-backed images override
    // reset() with truncate/extend so this is one filesystem operation.
    image.reset()?;

    let mut boot = [0u8; SECTOR_SIZE];
    boot[0..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    boot[3..11].copy_from_slice(b"MSDOS5.0");
    boot[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
    boot[13] = SECTORS_PER_CLUSTER;
    boot[14..16].copy_from_slice(&RESERVED_SECTORS.to_le_bytes());
    boot[16] = FAT_COUNT;
    boot[17..19].copy_from_slice(&ROOT_ENTRIES.to_le_bytes());
    boot[19..21].copy_from_slice(&0u16.to_le_bytes());
    boot[21] = 0xf8;
    boot[22..24].copy_from_slice(&g.sectors_per_fat.to_le_bytes());
    boot[24..26].copy_from_slice(&32u16.to_le_bytes()); // sectors per track
    boot[26..28].copy_from_slice(&64u16.to_le_bytes()); // heads
    boot[28..32].copy_from_slice(&0u32.to_le_bytes()); // hidden sectors
    boot[32..36].copy_from_slice(&total_sectors.to_le_bytes());
    boot[36] = 0x80; // fixed disk
    boot[38] = 0x29; // extended BPB signature
    boot[39..43].copy_from_slice(&0x5843_3430u32.to_le_bytes()); // volume ID
    boot[43..54].copy_from_slice(b"XCORE407   ");
    boot[54..62].copy_from_slice(b"FAT16   ");
    boot[510] = 0x55;
    boot[511] = 0xaa;
    image.write_at(0, &boot)?;

    let mut fat_header = [0u8; SECTOR_SIZE];
    fat_header[0..4].copy_from_slice(&[0xf8, 0xff, 0xff, 0xff]);
    let fat_bytes = g.sectors_per_fat as u64 * SECTOR_SIZE as u64;
    let fat1_offset = RESERVED_SECTORS as u64 * SECTOR_SIZE as u64;
    let fat2_offset = fat1_offset + fat_bytes;
    image.write_at(fat1_offset, &fat_header)?;
    image.write_at(fat2_offset, &fat_header)?;
    image.sync()?;
    Ok(g)
}
