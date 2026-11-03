//! Logical block device abstraction used by the SCSI layer.

pub const SECTOR_SIZE: usize = 512;

/// Errors returned by a storage backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    OutOfRange,
    ReadOnly,
    Io,
}

/// A synchronous 512-byte logical block device.
///
/// Implementations must not retain the supplied buffer. Calls are made from
/// USB class polling context, never from the OTG interrupt handler.
pub trait BlockDevice {
    fn sector_count(&self) -> u32;
    fn read_sector(&mut self, lba: u32, data: &mut [u8; SECTOR_SIZE])
        -> Result<(), BlockError>;
    fn write_sector(&mut self, lba: u32, data: &[u8; SECTOR_SIZE])
        -> Result<(), BlockError>;
    fn sync(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

/// Minimal RAM backend for protocol bring-up and host-side testing.
/// It is intentionally not the production NAND storage implementation.
pub struct RamDisk<const SECTORS: usize> {
    sectors: [[u8; SECTOR_SIZE]; SECTORS],
}

impl<const SECTORS: usize> RamDisk<SECTORS> {
    pub const fn new(fill: u8) -> Self {
        Self { sectors: [[fill; SECTOR_SIZE]; SECTORS] }
    }
}

impl<const SECTORS: usize> BlockDevice for RamDisk<SECTORS> {
    fn sector_count(&self) -> u32 { SECTORS as u32 }

    fn read_sector(&mut self, lba: u32, data: &mut [u8; SECTOR_SIZE])
        -> Result<(), BlockError>
    {
        let sector = self.sectors.get(lba as usize).ok_or(BlockError::OutOfRange)?;
        data.copy_from_slice(sector);
        Ok(())
    }

    fn write_sector(&mut self, lba: u32, data: &[u8; SECTOR_SIZE])
        -> Result<(), BlockError>
    {
        let sector = self.sectors.get_mut(lba as usize).ok_or(BlockError::OutOfRange)?;
        sector.copy_from_slice(data);
        Ok(())
    }
}
