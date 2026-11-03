# XCore407I USB MTP

Standalone Rust `no_std` firmware project for Windows file transfer to the
XCore407I / STM32F407IGTx.

## Target architecture

- USB 2.0 High-Speed device controller: STM32 OTG_HS with external USB3300 ULPI PHY.
- USB Still Image/PTP interface (class 06h, subclass 01h, protocol 01h), used by
  Windows Portable Devices / MTP.
- Two 512-byte bulk endpoints (IN/OUT) and one interrupt IN event endpoint.
- LittleFS remains the only on-device filesystem on the K9F1G08U0E NAND.
- MTP object handles map to LittleFS paths; files are streamed in bounded
  buffers rather than copied into a RAM disk.
- The emulator and MTP must not mutate LittleFS concurrently. Storage ownership
  must be handed off explicitly when the USB session starts and ends.

There is no FAT volume, disk image, SCSI layer, or USB Mass Storage interface.
Windows sees a portable device in File Explorer, not a drive letter, as with
an Android phone.

## USB implementation

The project reuses the existing `usb-device` class framework and
`synopsys-usb-otg` STM32 DWC2 controller driver from `rust-blinker`.
No UVC/isochronous endpoint scheduling or UVC incomplete-transfer recovery is
needed. The controller's internal DMA remains enabled; endpoint buffers and
transfer buffers must reside in DMA-accessible SRAM (not CCM).

## Current implementation status

Implemented:
- STM32F407 OTG_HS / external ULPI controller declaration and board ULPI pin mux.
- High-Speed PTP/MTP interface descriptor with bulk IN/OUT and interrupt IN endpoints.
- PTP/MTP container header codec and response-container encoder.
- Initial MTP command handling for `GetDeviceInfo`, `OpenSession`,
  `CloseSession`, and `GetStorageIDs` (currently reports zero mounted stores).
- A storage abstraction for bounded file streaming.

Bring-up firmware now exists (`src/main.rs`). It initializes the 168 MHz clock, configures the USB3300 ULPI pins, constructs the OTG_HS bus and MTP interface, and polls `usb-device`. It now handles the initial MTP identity/session exchange, but does not mount storage or expose files yet.

Not yet implemented:
- Complete MTP transaction state machine, data-phase recovery, and Windows interoperability.
- Device/storage/object-info datasets and object-handle/path database.
- LittleFS-backed implementation of `MtpStorage`.
- NAND ECC, bad-block management, and power-fail-safe LittleFS block device.
- MTP transaction state machine, USB reset/cancel handling, and hardware validation.

Do not connect this prototype to valuable NAND contents until the NAND driver,
LittleFS backend, and host-side interoperability tests are complete.

## Build

```sh
cargo build --release --target thumbv7em-none-eabihf
probe-rs download --chip STM32F407IGTx target/thumbv7em-none-eabihf/release/xcore407i_usb_mtp
```
