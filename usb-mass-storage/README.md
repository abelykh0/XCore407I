# XCore407I USB Mass Storage

Standalone Rust `no_std` firmware component for the XCore407I / STM32F407IGTx.

## Architecture

- USB 2.0 High-Speed device controller: STM32 OTG_HS with external ULPI PHY (USB3300).
- USB class: Mass Storage Class, SCSI transparent command set, Bulk-Only Transport.
- Logical block interface: 512-byte sectors.
- Planned persistent backend: K9F1G08U0E raw NAND managed by LittleFS.
- Host view: a preformatted FAT16 disk image stored as a single LittleFS file (`/volume.img`). MSC maps sectors directly to offsets in that file; no FAT/LittleFS translator is required.

USB MSC itself is filesystem-agnostic. Windows sees only a block device; it does not know that the persistent backend is LittleFS.

## Current implementation status

Implemented in this initial firmware slice:

- Rust crate and STM32F407 target configuration.
- OTG_HS / external ULPI PHY peripheral declaration with `HIGH_SPEED = true`.
- MSC interface descriptors (class 08h, subclass 06h, protocol 50h).
- Bulk-Only Transport CBW/CSW handling.
- SCSI INQUIRY, REQUEST SENSE, TEST UNIT READY, READ CAPACITY(10), READ(10),
  WRITE(10), MODE SENSE(6), START STOP UNIT, PREVENT/ALLOW and SYNCHRONIZE CACHE.
- A 512-byte logical block-device trait and a small RAM backend for protocol tests.
- A FAT16 formatter (96 MiB default, 2 KiB clusters) that initializes a disk image inside LittleFS.
- A LittleFS image-file adapter with a 4 KiB write-back cache.

Not yet implemented:

- ULPI GPIO and clock initialization / complete runnable firmware entry point.
- Production NAND driver with ECC, factory/runtime bad-block management, and power-fail-safe block mapping.
- Production NAND `littlefs2::driver::Storage` implementation and mount configuration.
- Firmware-side FatFs disk I/O adapter to access the same FAT image after USB disconnect.
- Host interoperability and hardware validation.

The existing `rust-blinker/src/nand.rs` is a destructive single-block bring-up test, not a reusable storage driver. It is intentionally not copied or called here.

## Important storage ownership rule

The firmware must give the host exclusive ownership while MSC is connected. The emulator must not access LittleFS concurrently. On host disconnect, synchronize and unmount/reopen the filesystem before handing storage back to the emulator.

## Memory budget

The BOT implementation stages one 512-byte sector. The LittleFS image adapter uses a 4 KiB cache. It does not allocate a RAM disk. The generic RAM backend is only for tests and must not be instantiated at full NAND capacity on the target.

## Build target

```sh
rustup target add thumbv7em-none-eabihf
cargo check --lib --target thumbv7em-none-eabihf
```

A runnable board image, production NAND driver, and host-side BOT/FAT interoperability tests are follow-up milestones. Do not connect this prototype to valuable NAND contents until the production NAND and filesystem layers are complete.
