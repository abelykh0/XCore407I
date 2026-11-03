# XCore407I USB MTP

Standalone Rust `no_std` firmware for exposing the XCore407I / STM32F407IGTx
NAND filesystem to a Windows host through USB Media Transfer Protocol (MTP).

## Architecture

- USB 2.0 High-Speed device using STM32 OTG_HS and the external USB3300 ULPI PHY.
- USB Still Image/PTP interface (class 06h, subclass 01h, protocol 01h), with
  bulk IN/OUT and interrupt IN endpoints.
- LittleFS on the NAND remains the on-device filesystem. There is no FAT volume,
  disk image, SCSI layer, or USB Mass Storage interface.
- MTP object handles refer to a bounded in-RAM snapshot of the LittleFS root
  directory. File contents are streamed between USB and LittleFS; the firmware
  does not build a RAM disk or buffer an entire file.
- USB polling runs from TIM2. The interrupt owns USB protocol state; NAND and
  LittleFS operations run in the main context through bounded request/data
  handoffs. LittleFS is never accessed from the USB polling interrupt.
- USB DMA buffers must remain in DMA-accessible SRAM, not CCM.

Windows should enumerate this as a portable MTP device in File Explorer, not as
a drive letter.

## Implemented

### USB and session handling

- OTG_HS / ULPI board initialization and High-Speed PTP/MTP interface descriptors.
- Bulk and interrupt endpoint setup, MTP container framing, and response encoding.
- TIM2-driven USB polling and serialized MTP transaction handling.
- Device identity, session open/close, storage ID/info, object count, and handle
  enumeration.

### LittleFS-backed storage

- Mounts the existing NAND LittleFS volume and keeps the mounted filesystem
  available while USB requests are serviced.
- Enumerates root-directory entries into a bounded snapshot
  (`MTP_OBJECT_CAPACITY`, currently 512 objects).
- Reads file data from LittleFS in bounded chunks and streams it to the host.
- Receives uploads in bounded chunks and writes them to a temporary LittleFS
  file before finalizing the destination.
- Supports root-level file creation, deletion, and rename.
- Seeds `MTP-README.txt`, `HELLO.txt`, and `PATTERN.BIN` only when absent;
  verifies the `HELLO.txt` read path at startup.
- Exposes a virtual `MTP-STATUS.txt` object with live diagnostic counters.

### Object metadata and properties

- Implements `GetObjectInfo`, `GetObjectPropsSupported`,
  `GetObjectPropDesc`, `GetObjectPropList`, and `SetObjectPropValue`.
- Supports the advertised storage ID, object format, protection status,
  compressed size, filename, parent handle, persistent unique identifier, and
  name properties. Filename and name writes are used for rename.
- Persists 128-bit MTP Persistent Unique Object Identifiers (PUIDs) as LittleFS
  user attributes. The identifier combines the STM32 factory UID with a
  monotonically allocated counter. Rename preserves the file's attribute/PUID.
- Implements `GetObjectPropValue` for the advertised properties. This operation
  was added most recently and still requires hardware regression testing against
  Windows enumeration and file reads.

## Remaining work

### Correctness and interoperability

- **Validate the new `GetObjectPropValue` implementation on hardware.** Check
  enumeration, property queries, opening, and copying files. Earlier versions
  of this operation caused Windows to lose the object listing; revert or fix
  this operation if that regression returns.
- Complete the `GetObjectPropList` parameter semantics, including object
  selection, property-group handling, depth, and correct results for requests
  covering all objects. Current handling is intentionally narrower than the
  full MTP specification.
- Review every implemented operation's parameter validation, response codes,
  data-phase sequencing, and behavior on malformed or interrupted transfers.
- Add and validate MTP event notifications (for example, object added,
  removed, and information changed), or explicitly document host-refresh
  behavior if events remain unsupported.
- Implement protocol cancellation/reset and recovery paths, including USB bus
  reset, aborted transactions, and host disconnect/reconnect. Confirm the
  current firmware recovers cleanly without requiring a power cycle.
- Test with more than one MTP host implementation in addition to Windows, if
  practical.

### Storage model and file operations

- Extend the current root-only object snapshot/path mapping to nested
  directories, including correct parent handles, directory traversal, and
  directory creation/removal where supported.
- Decide and implement a coherent refresh/invalidation strategy for the object
  snapshot after create, delete, and rename operations. Ensure handles remain
  valid and metadata/storage statistics stay consistent after mutations.
- Define behavior for names or paths that cannot be represented by the current
  fixed-size UTF-8 object-name field; test Unicode filenames and boundary
  lengths end to end.
- Test power loss and NAND/LittleFS errors during upload finalization, rename,
  deletion, and PUID allocation. Confirm temporary upload files are handled
  safely after reboot.
- Verify free-space and capacity reporting after uploads and deletions, rather
  than relying only on startup values.

### Optional MTP features

The following are not currently advertised as supported operations and should
only be added when their full data/response phases are implemented and tested:

- `GetPartialObject` for range-based reads.
- `GetObjectReferences` and reference management, if required by a host.
- Device-property operations and event support, if a host requires them.
- Additional object properties, formats, timestamps, thumbnails, and metadata
  only when the firmware can return truthful values.

## Known scope and limitations

- The current object snapshot is bounded and root-directory oriented; it is not
  yet a general recursive filesystem browser.
- Upload, rename, and delete support is currently intended for root-level files.
- The firmware advertises only the MTP operations implemented in its command
  dispatcher. Do not advertise additional operations merely to improve host
  discovery.
- RTC-backed timestamps are not implemented. Do not fabricate modification
  dates or other metadata that the filesystem does not currently provide.
- This is an MTP file-transfer implementation, not a USB Mass Storage device.

## Build and flash

```sh
cargo build --release --target thumbv7em-none-eabihf
probe-rs download --chip STM32F407IGTx target/thumbv7em-none-eabihf/release/xcore407i_usb_mtp
```
