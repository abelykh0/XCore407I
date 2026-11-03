//! Cached LittleFS file adapter for the fixed-size FAT disk image.
//!
//! The image file is kept open only for the duration of a closure-based
//! operation, following littlefs2's safe API. A 4 KiB write-back cache groups
//! eight 512-byte MSC sectors and avoids opening/syncing LittleFS for every
//! individual sector write.

use crate::block::BlockError;
use crate::image::ImageFile;
use crate::fat16::{self, Fat16Geometry};
use littlefs2::driver::Storage;
use littlefs2::fs::Filesystem;
use littlefs2::io::prelude::{Read, Seek, Write};
use littlefs2::io::SeekFrom;

const CACHE_BYTES: usize = 4096;
const IMAGE_PATH: &littlefs2::path::Path = littlefs2::path!("/volume.img");

pub struct LittleFsImage<'fs, 'storage, S: Storage> {
    fs: &'fs Filesystem<'storage, S>,
    image_bytes: u64,
    cached_block: Option<u32>,
    dirty: bool,
    cache: [u8; CACHE_BYTES],
}

impl<'fs, 'storage, S: Storage> LittleFsImage<'fs, 'storage, S> {
    pub fn new(fs: &'fs Filesystem<'storage, S>, image_bytes: u64) -> Self {
        Self {
            fs, image_bytes, cached_block: None, dirty: false,
            cache: [0; CACHE_BYTES],
        }
    }

    /// Create the image file on first boot and extend it to the fixed volume
    /// size. LittleFS zero-fills the extended range. Existing files are left
    /// untouched; call fat16::format to reinitialize an existing volume.
    pub fn create_image_file(
        fs: &Filesystem<'storage, S>,
        image_bytes: u64,
    ) -> Result<(), BlockError> {
        if image_bytes == 0 || image_bytes > usize::MAX as u64 {
            return Err(BlockError::OutOfRange);
        }
        fs.create_file_and_then(IMAGE_PATH, |file| {
            file.set_len(image_bytes as usize)?;
            file.sync()
        }).map_err(|_| BlockError::Io)
    }

    /// Ensure the FAT16 image exists. A newly created image is formatted;
    /// an existing image is preserved so Windows can retain its files.
    pub fn ensure_volume(
        fs: &Filesystem<'storage, S>,
        total_sectors: u32,
    ) -> Result<(bool, Fat16Geometry), BlockError> {
        let bytes = total_sectors as u64 * 512;
        let created = !fs.exists(IMAGE_PATH);
        if created {
            Self::create_image_file(fs, bytes)?;
            let mut image = Self::new(fs, bytes);
            let geometry = fat16::format(&mut image, total_sectors)?;
            Ok((true, geometry))
        } else {
            Ok((false, fat16::geometry(total_sectors)?))
        }
    }

    fn check_range(&self, offset: u64, len: usize) -> Result<(), BlockError> {
        let end = offset.checked_add(len as u64).ok_or(BlockError::OutOfRange)?;
        if end > self.image_bytes { return Err(BlockError::OutOfRange); }
        Ok(())
    }

    fn read_file(
        fs: &Filesystem<'storage, S>,
        offset: u64,
        data: &mut [u8],
    ) -> Result<(), BlockError> {
        fs.open_file_with_options_and_then(
            |options| options.read(true).write(false),
            IMAGE_PATH,
            |file| {
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(data)?;
                Ok(())
            },
        ).map_err(|_| BlockError::Io)
    }

    fn write_file(
        fs: &Filesystem<'storage, S>,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BlockError> {
        fs.open_file_with_options_and_then(
            |options| options.read(true).write(true),
            IMAGE_PATH,
            |file| {
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(data)?;
                file.sync()?;
                Ok(())
            },
        ).map_err(|_| BlockError::Io)
    }

    fn flush_cache(&mut self) -> Result<(), BlockError> {
        if self.dirty {
            let block = self.cached_block.ok_or(BlockError::Io)?;
            Self::write_file(self.fs, block as u64 * CACHE_BYTES as u64, &self.cache)?;
            self.dirty = false;
        }
        Ok(())
    }

    fn load_block(&mut self, block: u32) -> Result<(), BlockError> {
        if self.cached_block == Some(block) { return Ok(()); }
        self.flush_cache()?;
        Self::read_file(self.fs, block as u64 * CACHE_BYTES as u64, &mut self.cache)?;
        self.cached_block = Some(block);
        self.dirty = false;
        Ok(())
    }
}

impl<S: Storage> ImageFile for LittleFsImage<'_, '_, S> {
    fn len(&self) -> u64 { self.image_bytes }

    fn read_at(&mut self, offset: u64, data: &mut [u8]) -> Result<(), BlockError> {
        self.check_range(offset, data.len())?;
        let mut done = 0;
        while done < data.len() {
            let absolute = offset + done as u64;
            let block = (absolute / CACHE_BYTES as u64) as u32;
            let within = (absolute % CACHE_BYTES as u64) as usize;
            let count = (data.len() - done).min(CACHE_BYTES - within);
            self.load_block(block)?;
            data[done..done + count].copy_from_slice(&self.cache[within..within + count]);
            done += count;
        }
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        self.check_range(offset, data.len())?;
        let mut done = 0;
        while done < data.len() {
            let absolute = offset + done as u64;
            let block = (absolute / CACHE_BYTES as u64) as u32;
            let within = (absolute % CACHE_BYTES as u64) as usize;
            let count = (data.len() - done).min(CACHE_BYTES - within);
            self.load_block(block)?;
            self.cache[within..within + count].copy_from_slice(&data[done..done + count]);
            self.dirty = true;
            done += count;
        }
        Ok(())
    }

    fn sync(&mut self) -> Result<(), BlockError> {
        self.flush_cache()
    }

    fn reset(&mut self) -> Result<(), BlockError> {
        let size = self.image_bytes as usize;
        self.fs.open_file_with_options_and_then(
            |options| options.read(true).write(true),
            IMAGE_PATH,
            |file| {
                file.set_len(0)?;
                file.set_len(size)?;
                file.sync()
            },
        ).map_err(|_| BlockError::Io)?;
        self.cached_block = None;
        self.dirty = false;
        Ok(())
    }
}
