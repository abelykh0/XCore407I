//! Sector-addressable view of a file stored by the persistent filesystem.
//!
//! The image file contains a complete FAT16 volume. MSC exports this file as
//! raw sectors; LittleFS stores the file itself and therefore remains the
//! only filesystem used directly on the NAND device.

use crate::block::{BlockDevice, BlockError, SECTOR_SIZE};

pub trait ImageFile {
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, data: &mut [u8]) -> Result<(), BlockError>;
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError>;
    fn sync(&mut self) -> Result<(), BlockError>;

    /// Clear the complete image while preserving its logical size.
    fn reset(&mut self) -> Result<(), BlockError> {
        let zeros = [0u8; 4096];
        let mut offset = 0u64;
        while offset < self.len() {
            let n = (self.len() - offset).min(zeros.len() as u64) as usize;
            self.write_at(offset, &zeros[..n])?;
            offset += n as u64;
        }
        self.sync()
    }
}

pub struct ImageBlockDevice<F: ImageFile> {
    file: F,
    sectors: u32,
}

impl<F: ImageFile> ImageBlockDevice<F> {
    pub fn new(file: F) -> Result<Self, BlockError> {
        let bytes = file.len();
        if bytes == 0 || bytes % SECTOR_SIZE as u64 != 0 || bytes / SECTOR_SIZE as u64 > u32::MAX as u64 {
            return Err(BlockError::Io);
        }
        Ok(Self { file, sectors: (bytes / SECTOR_SIZE as u64) as u32 })
    }

    pub fn image_mut(&mut self) -> &mut F { &mut self.file }
    pub fn into_inner(self) -> F { self.file }
}

impl<F: ImageFile> BlockDevice for ImageBlockDevice<F> {
    fn sector_count(&self) -> u32 { self.sectors }

    fn read_sector(&mut self, lba: u32, data: &mut [u8; SECTOR_SIZE]) -> Result<(), BlockError> {
        if lba >= self.sectors { return Err(BlockError::OutOfRange); }
        self.file.read_at(lba as u64 * SECTOR_SIZE as u64, data)
    }

    fn write_sector(&mut self, lba: u32, data: &[u8; SECTOR_SIZE]) -> Result<(), BlockError> {
        if lba >= self.sectors { return Err(BlockError::OutOfRange); }
        self.file.write_at(lba as u64 * SECTOR_SIZE as u64, data)
    }

    fn sync(&mut self) -> Result<(), BlockError> { self.file.sync() }
}
