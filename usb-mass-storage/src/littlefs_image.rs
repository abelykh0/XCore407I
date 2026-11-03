//! Adapter exposing a fixed-size LittleFS file as a seekable disk image.
//!
//! The closure-based LittleFS API opens and closes the file for each operation.
//! This is safe (no leaked LittleFS file handles) and keeps RAM bounded, but
//! is intentionally a first implementation; sector-write batching should be
//! added after the BOT command lifecycle is wired to a persistent backend.

use crate::block::BlockError;
use crate::image::ImageFile;
use littlefs2::driver::Storage;
use littlefs2::fs::Filesystem;
use littlefs2::io::prelude::{Read, Seek, Write};
use littlefs2::io::SeekFrom;

const IMAGE_PATH: &littlefs2::path::Path = littlefs2::path!("/volume.img");

pub struct LittleFsImage<'fs, 'storage, S: Storage> {
    fs: &'fs Filesystem<'storage, S>,
    image_bytes: u64,
}

impl<'fs, 'storage, S: Storage> LittleFsImage<'fs, 'storage, S> {
    pub fn new(fs: &'fs Filesystem<'storage, S>, image_bytes: u64) -> Self {
        Self { fs, image_bytes }
    }

    fn check_range(&self, offset: u64, len: usize) -> Result<(), BlockError> {
        let end = offset.checked_add(len as u64).ok_or(BlockError::OutOfRange)?;
        if end > self.image_bytes { return Err(BlockError::OutOfRange); }
        Ok(())
    }
}

impl<S: Storage> ImageFile for LittleFsImage<'_, '_, S> {
    fn len(&self) -> u64 { self.image_bytes }

    fn read_at(&mut self, offset: u64, data: &mut [u8]) -> Result<(), BlockError> {
        self.check_range(offset, data.len())?;
        self.fs.open_file_with_options_and_then(
            |options| options.read(true).write(false),
            IMAGE_PATH,
            |file| {
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(data)?;
                Ok(())
            },
        ).map_err(|_| BlockError::Io)
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        self.check_range(offset, data.len())?;
        self.fs.open_file_with_options_and_then(
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

    fn sync(&mut self) -> Result<(), BlockError> {
        self.fs.open_file_with_options_and_then(
            |options| options.read(true).write(true),
            IMAGE_PATH,
            |file| file.sync(),
        ).map_err(|_| BlockError::Io)
    }
}
