#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use cortex_m::peripheral::NVIC;
use cortex_m_rt::entry;
use panic_halt as _;
use littlefs2::path;
use littlefs2::path::PathBuf;
use littlefs2::io::Read as LittleFsRead;
use littlefs2::io::Write as LittleFsWrite;
use stm32f4::stm32f407::{interrupt, Interrupt};
use stm32f4xx_hal::pac;
use synopsys_usb_otg::UsbBus;
use usb_device::descriptor::lang_id::LangID;
use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};

use xcore407i_usb_mtp::{
    board,
    bus::usb_bus,
    mtp::{MtpObject, MtpUsbClass, UploadObjectInfo, MTP_OBJECT_CAPACITY},
    nand_fs,
    usb_hs::F407UsbHs,
};

const USB_POLL_HZ: u32 = 8_000;
const TIM2_PRIORITY: u8 = 2;

// LittleFS user attributes reserved for persistent MTP object identifiers.
const MTP_PUID_ATTRIBUTE_ID: u8 = b'P';
const MTP_PUID_COUNTER_ATTRIBUTE_ID: u8 = b'C';
const MTP_PUID_MAX_COUNTER: u32 = 0xffff_fffd;
const MTP_STATUS_PUID_COUNTER: u32 = 0xffff_fffe;

// Deterministic 2 KiB payload for testing multi-packet MTP reads.
const fn make_test_pattern() -> [u8; 2048] {
    let mut bytes = [0u8; 2048];
    let mut i = 0;
    while i < bytes.len() {
        bytes[i] = (i as u8).wrapping_mul(37).wrapping_add(11);
        i += 1;
    }
    bytes
}
static TEST_PATTERN: [u8; 2048] = make_test_pattern();

type BoardUsbBus = UsbBus<F407UsbHs>;
type BoardUsbDevice = UsbDevice<'static, BoardUsbBus>;
type BoardMtpClass = MtpUsbClass<'static, BoardUsbBus>;

// Initialized and published before TIM2 is enabled. TIM2 is the sole owner
// of mutable USB-device/class access while polling.
static USB_DEVICE_PTR: AtomicPtr<BoardUsbDevice> = AtomicPtr::new(ptr::null_mut());
static MTP_PTR: AtomicPtr<BoardMtpClass> = AtomicPtr::new(ptr::null_mut());

#[unsafe(link_section = ".ccmram.usb_device")]
static mut USB_DEVICE: MaybeUninit<BoardUsbDevice> = MaybeUninit::uninit();
static mut MTP: MaybeUninit<BoardMtpClass> = MaybeUninit::uninit();

// Large filesystem and upload buffers live in main SRAM, not on the stack.
static mut OBJECTS: [MtpObject; MTP_OBJECT_CAPACITY] = [MtpObject::EMPTY; MTP_OBJECT_CAPACITY];
static mut UPLOAD_STAGING: [u8; nand_fs::PAGE_SIZE] = [0; nand_fs::PAGE_SIZE];
// GET_OBJECT staging fills the 4 KiB MTP transport buffer in one batch.
static mut GET_OBJECT_STAGING: [u8; 4084] = [0; 4084];

#[entry]
fn main() -> ! {
    board::configure_system_clock_168mhz();

    let peripherals = pac::Peripherals::take().unwrap();
    let mut core = cortex_m::Peripherals::take().unwrap();
    let mut rcc = peripherals.RCC;

    board::configure_ulpi(
        &mut rcc,
        peripherals.GPIOA,
        peripherals.GPIOB,
        peripherals.GPIOC,
        peripherals.GPIOH,
        peripherals.GPIOI,
    );
    board::configure_nand(&mut rcc, peripherals.GPIOD, peripherals.GPIOE);

    // Start USB polling before NAND initialization/formatting. TIM2 continues
    // servicing USB while the main thread accesses NAND.
    let bus = usb_bus().expect("USB HS bus already initialized");
    let mtp_ptr = unsafe { (*ptr::addr_of_mut!(MTP)).as_mut_ptr() };
    unsafe {
        mtp_ptr.write(MtpUsbClass::new(bus));
        (*mtp_ptr).set_readme(b"Initializing NAND LittleFS...\\r\\n");
    }

    let strings = [
        StringDescriptors::new(LangID::EN_US)
            .manufacturer("XCore407I")
            .product("XCore407I NAND")
            .serial_number("XCORE407I-002"),
    ];
    let usb_ptr = unsafe { (*ptr::addr_of_mut!(USB_DEVICE)).as_mut_ptr() };
    unsafe {
        usb_ptr.write(
            UsbDeviceBuilder::new(bus, UsbVidPid(0x1209, 0x4071))
                .strings(&strings)
                .unwrap()
                .max_packet_size_0(64)
                .unwrap()
                .build(),
        );
    }

    MTP_PTR.store(mtp_ptr, Ordering::Release);
    USB_DEVICE_PTR.store(usb_ptr, Ordering::Release);
    configure_get_object_timer(&rcc, &peripherals.TIM5);
    configure_usb_poll_timer(&rcc, &peripherals.TIM2, &mut core.NVIC);

    // Mount the existing volume once. The report is generated from this
    // mount, then the same mounted filesystem remains alive while the main
    // context services future MTP requests. No NAND/LittleFS work runs in TIM2.
    let mounted = nand_fs::with_mounted_report::<()>(|fs, report, report_len| {
        // Seed a few real files into the existing LittleFS volume. Each file
        // is created only when absent, so normal boots never overwrite user
        // data. These writes happen in main context, never in TIM2.
        seed_test_files(fs)?;

        // Verify the storage read path independently of USB/MTP. This file is
        // refreshed on each boot and is visible in the root directory.
        let read_test = test_hello_readback(fs);
        if read_test.starts_with(b"LittleFS readback: PASS") {
            fs.write(path!("/MTP-DIAG-PASS.txt"), read_test)?;
        } else {
            fs.write(path!("/MTP-DIAG-FAIL.txt"), read_test)?;
        }

        // Build a bounded snapshot of the LittleFS root directory in main
        // context. Directory entries are copied out; no LittleFS object is
        // accessed by the USB polling interrupt.
        let capacity_bytes = fs.total_space() as u64;
        let free_bytes = fs.available_space().unwrap_or(0) as u64;
        let objects = unsafe { &mut *ptr::addr_of_mut!(OBJECTS) };
        objects.fill(MtpObject::EMPTY);
        let mut object_count = 0usize;
        fs.read_dir_and_then(path!("/"), |entries| {
            for entry in entries.skip(2) {
                let entry = entry?;
                if object_count == objects.len() {
                    break;
                }
                let name = entry.file_name().as_ref().as_bytes();
                // The MTP snapshot stores UTF-8 names in a fixed 64-byte
                // field. Never truncate a multibyte character or publish an
                // invalid UTF-8 filename to the PTP string encoder.
                if name.is_empty() || name.len() > objects[object_count].name.len()
                    || core::str::from_utf8(name).is_err()
                {
                    continue;
                }
                let name_len = name.len();
                objects[object_count].name[..name_len].copy_from_slice(name);
                objects[object_count].name_len = name_len as u8;
                objects[object_count].handle = object_count as u32 + 1;
                let metadata = entry.metadata();
                objects[object_count].size = metadata.len().min(u32::MAX as usize) as u32;
                objects[object_count].is_dir = metadata.is_dir();
                object_count += 1;
            }
            Ok(())
        })?;

        // Persistent object identifiers are stored as LittleFS attributes.
        // Reconcile the allocator with existing attributes before assigning
        // identifiers to files created before this feature was installed.
        let device_uid = read_device_uid()?;
        ensure_puid_counter(fs, objects, object_count, &device_uid)?;
        for index in 0..object_count {
            let name = core::str::from_utf8(
                &objects[index].name[..objects[index].name_len as usize],
            )
            .map_err(|_| littlefs2::io::Error::INVALID)?;
            let relative_path = PathBuf::try_from(name)
                .map_err(|_| littlefs2::io::Error::INVALID)?;
            let full_path = path!("/").join(relative_path.as_path());
            let mut puid = load_or_create_puid(fs, &full_path, &device_uid)?;
            if objects[..index].iter().any(|previous| previous.puid == puid) {
                // A copied or otherwise duplicated attribute must not expose
                // the same persistent identifier for two distinct objects.
                puid = allocate_puid(fs, &full_path, &device_uid)?;
            }
            objects[index].puid = puid;
        }

        // The special in-memory README response contains the NAND report,
        // not the seeded LittleFS README text. Make ObjectInfo advertise the
        // exact length returned by that response.
        for object in &mut objects[..object_count] {
            if object.name_len as usize == 14
                && &object.name[..14] == b"MTP-README.txt"
            {
                object.size = report_len as u32;
            }
        }

        // A virtual status object exposes live USB/MTP counters without
        // writing diagnostics to NAND. Its payload is generated on each read.
        if object_count < objects.len() {
            let status = &mut objects[object_count];
            status.name[..14].copy_from_slice(b"MTP-STATUS.txt");
            status.name_len = 14;
            status.handle = object_count as u32 + 1;
            status.size = 480;
            status.is_dir = false;
            status.puid = make_puid(&device_uid, MTP_STATUS_PUID_COUNTER);
            object_count += 1;
        }

        // Publish the diagnostic text and root snapshot atomically relative
        // to TIM2. Handles are 1..N and refer to this snapshot.
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        let _ = mtp.set_readme(&report[..report_len]);
        mtp.set_objects(&objects[..object_count]);
        mtp.set_storage_stats(capacity_bytes, free_bytes);
        peripherals.TIM2.sr().write(|w| w.uif().clear_bit());
        unsafe {
            NVIC::unpend(Interrupt::TIM2);
            NVIC::unmask(Interrupt::TIM2);
        }

        let mut next_object_handle = object_count as u32 + 1;
        let mut pending_upload: Option<(UploadObjectInfo, u32, usize)> = None;
        let mut upload_buffer = [0u8; 512];
        // Only main context accesses this staging buffer; keep it out of the
        // relatively small exception/main stack.
        let upload_staging = unsafe { &mut *ptr::addr_of_mut!(UPLOAD_STAGING) };
        let mut upload_staging_len = 0usize;

        loop {
            // Pull commands out of the USB ISR and perform all LittleFS work
            // here, with TIM2 free to keep servicing USB HS.
            NVIC::mask(Interrupt::TIM2);
            let (get_request, delete_request, rename_request, info_request, upload_data, upload_abort) = {
                let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
                (
                    mtp.take_get_object_request(),
                    mtp.take_delete_object_request(),
                    mtp.take_rename_object_request(),
                    mtp.take_send_object_info_request(),
                    mtp.take_upload_data(&mut upload_buffer),
                    mtp.take_upload_abort(),
                )
            };
            peripherals.TIM2.sr().write(|w| w.uif().clear_bit());
            unsafe {
                NVIC::unpend(Interrupt::TIM2);
                NVIC::unmask(Interrupt::TIM2);
            }

            if upload_abort {
                pending_upload = None;
                upload_staging_len = 0;
                let temp_path = path!("/.mtp-upload.tmp");
                if fs.exists(&temp_path) {
                    let _ = fs.remove(&temp_path);
                }
            }

            if let Some((transaction_id, info)) = info_request {
                service_send_object_info(
                    fs,
                    objects,
                    object_count,
                    info,
                    transaction_id,
                    &mut next_object_handle,
                    &mut pending_upload,
                );
            } else if let Some((transaction_id, length, final_packet)) = upload_data {
                service_upload_data(
                    fs,
                    &upload_buffer,
                    length,
                    objects,
                    &mut object_count,
                    transaction_id,
                    final_packet,
                    &mut pending_upload,
                    &mut *upload_staging,
                    &mut upload_staging_len,
                );
            } else if let Some((handle, transaction_id, name, name_len)) = rename_request {
                service_rename_object(fs, objects, &mut object_count, handle, &name[..name_len as usize], transaction_id);
            } else if let Some((handle, transaction_id)) = delete_request {
                service_delete_object(
                    fs,
                    objects,
                    &mut object_count,
                    handle,
                    transaction_id,
                );
            } else if let Some((handle, transaction_id)) = get_request {
                service_get_object(fs, objects, object_count, handle, transaction_id);
            } else {
                core::hint::spin_loop();
            }
        }
    });
    let _ = mounted;
    loop {
        core::hint::spin_loop();
    }
}



/// Read the STM32F407's factory-programmed 96-bit unique device identifier.
fn read_device_uid() -> littlefs2::io::Result<[u8; 12]> {
    let words = unsafe {
        [
            ptr::read_volatile(0x1fff_7a10 as *const u32),
            ptr::read_volatile(0x1fff_7a14 as *const u32),
            ptr::read_volatile(0x1fff_7a18 as *const u32),
        ]
    };
    let mut uid = [0u8; 12];
    for (index, word) in words.iter().enumerate() {
        uid[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    if uid.iter().all(|byte| *byte == 0) || uid.iter().all(|byte| *byte == 0xff) {
        return Err(littlefs2::io::Error::CORRUPTION);
    }
    Ok(uid)
}

/// Compose a 128-bit MTP identifier from the device UID and allocation number.
fn make_puid(device_uid: &[u8; 12], counter: u32) -> [u8; 16] {
    let mut puid = [0u8; 16];
    puid[..12].copy_from_slice(device_uid);
    puid[12..].copy_from_slice(&counter.to_le_bytes());
    puid
}

/// Ensure the root-directory allocation counter cannot reuse an identifier
/// already present on disk, including after a copied or repaired filesystem.
fn ensure_puid_counter(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    objects: &[MtpObject],
    object_count: usize,
    device_uid: &[u8; 12],
) -> littlefs2::io::Result<()> {
    let mut highest_existing = 0u32;
    let mut stored_counter = [0u8; 4];

    for object in &objects[..object_count] {
        let name = core::str::from_utf8(&object.name[..object.name_len as usize])
            .map_err(|_| littlefs2::io::Error::INVALID)?;
        let relative_path = PathBuf::try_from(name)
            .map_err(|_| littlefs2::io::Error::INVALID)?;
        let full_path = path!("/").join(relative_path.as_path());
        let mut stored_puid = [0u8; 16];
        if let Some(attribute) = fs.attribute(&full_path, MTP_PUID_ATTRIBUTE_ID, &mut stored_puid)? {
            if attribute.total_size() != 16 || attribute.data().len() != 16 {
                return Err(littlefs2::io::Error::CORRUPTION);
            }
            if &stored_puid[..12] == device_uid {
                let counter = u32::from_le_bytes(stored_puid[12..16].try_into().unwrap());
                if counter == 0 || counter > MTP_PUID_MAX_COUNTER {
                    return Err(littlefs2::io::Error::CORRUPTION);
                }
                highest_existing = highest_existing.max(counter);
            }
        }
    }

    let (stored, counter_missing) = match fs.attribute(
        path!("/"),
        MTP_PUID_COUNTER_ATTRIBUTE_ID,
        &mut stored_counter,
    )? {
        Some(attribute) if attribute.total_size() == 4 && attribute.data().len() == 4 => {
            (u32::from_le_bytes(stored_counter), false)
        }
        Some(_) => return Err(littlefs2::io::Error::CORRUPTION),
        None => (1, true),
    };

    if stored == 0 || stored > MTP_PUID_MAX_COUNTER + 1 {
        return Err(littlefs2::io::Error::CORRUPTION);
    }
    let minimum_next = highest_existing
        .checked_add(1)
        .ok_or(littlefs2::io::Error::NO_SPACE)?;
    let next = stored.max(minimum_next);
    if next > MTP_PUID_MAX_COUNTER + 1 {
        return Err(littlefs2::io::Error::NO_SPACE);
    }
    if counter_missing || next != stored {
        fs.set_attribute(
            path!("/"),
            MTP_PUID_COUNTER_ATTRIBUTE_ID,
            &next.to_le_bytes(),
        )?;
    }
    Ok(())
}

/// Allocate and persist the next identifier before writing it to the object.
/// A power loss can consume a number, but cannot cause that number to be reused.
fn allocate_puid(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    object_path: &littlefs2::path::Path,
    device_uid: &[u8; 12],
) -> littlefs2::io::Result<[u8; 16]> {
    let mut counter_bytes = [0u8; 4];
    let counter = match fs.attribute(
        path!("/"),
        MTP_PUID_COUNTER_ATTRIBUTE_ID,
        &mut counter_bytes,
    )? {
        Some(attribute) if attribute.total_size() == 4 && attribute.data().len() == 4 => {
            u32::from_le_bytes(counter_bytes)
        }
        _ => return Err(littlefs2::io::Error::CORRUPTION),
    };
    if counter == 0 || counter > MTP_PUID_MAX_COUNTER {
        return Err(littlefs2::io::Error::NO_SPACE);
    }

    let next = counter + 1;
    fs.set_attribute(
        path!("/"),
        MTP_PUID_COUNTER_ATTRIBUTE_ID,
        &next.to_le_bytes(),
    )?;
    let puid = make_puid(device_uid, counter);
    fs.set_attribute(object_path, MTP_PUID_ATTRIBUTE_ID, &puid)?;
    Ok(puid)
}

/// Load an existing identifier or allocate one for a file created before MTP
/// PUID support was added.
fn load_or_create_puid(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    object_path: &littlefs2::path::Path,
    device_uid: &[u8; 12],
) -> littlefs2::io::Result<[u8; 16]> {
    let mut puid = [0u8; 16];
    match fs.attribute(object_path, MTP_PUID_ATTRIBUTE_ID, &mut puid)? {
        Some(attribute) if attribute.total_size() == 16 && attribute.data().len() == 16 => {
            Ok(puid)
        }
        Some(_) => Err(littlefs2::io::Error::CORRUPTION),
        None => allocate_puid(fs, object_path, device_uid),
    }
}

/// Create test content once, without replacing existing files.
fn seed_test_files(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
) -> littlefs2::io::Result<()> {
    if !fs.exists(path!("/MTP-README.txt")) {
        fs.write(
            path!("/MTP-README.txt"),
            b"XCore407I MTP / LittleFS test files\r\n\r\n\
These files are stored on the NAND chip in the existing LittleFS volume.\r\n\
They are exposed to the host through USB High Speed MTP.\r\n\r\n\
HELLO.txt is a small text read test.\r\n\
PATTERN.BIN is a 2048-byte deterministic binary test pattern.\r\n\
EMPTY.TXT is intentionally zero bytes long.\r\n\r\n\
PATTERN.BIN byte[n] = (n * 37 + 11) modulo 256.\r\n",
        )?;
    }
    if !fs.exists(path!("/HELLO.txt")) {
        fs.write(
            path!("/HELLO.txt"),
            b"Hello from XCore407I!\r\n\
This file is stored in NAND using LittleFS and read over USB MTP.\r\n",
        )?;
    }
    if !fs.exists(path!("/PATTERN.BIN")) {
        fs.write(path!("/PATTERN.BIN"), &TEST_PATTERN)?;
    }
    Ok(())
}

/// Read HELLO.txt directly from LittleFS and return a short persistent
/// diagnostic. This separates NAND/LittleFS read failures from USB transport.
fn test_hello_readback(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
) -> &'static [u8] {
    const EXPECTED: &[u8] = b"Hello from XCore407I!\r\nThis file is stored in NAND using LittleFS and read over USB MTP.\r\n";
    let mut actual = [0u8; 128];
    let mut total = 0usize;
    let result = fs.open_file_and_then(path!("/HELLO.txt"), |file| {
        while total < EXPECTED.len() {
            let count = file.read(&mut actual[total..EXPECTED.len()])?;
            if count == 0 {
                break;
            }
            total += count;
        }
        Ok(())
    });

    if result.is_err() {
        return b"LittleFS readback: ERROR opening/reading HELLO.txt\r\n";
    }
    if total != EXPECTED.len() {
        return b"LittleFS readback: SHORT READ from HELLO.txt\r\n";
    }
    if &actual[..total] != EXPECTED {
        return b"LittleFS readback: DATA MISMATCH in HELLO.txt\r\n";
    }
    b"LittleFS readback: PASS (HELLO.txt, 90 bytes)\r\n"
}

/// Reserve a new root-level upload destination. SendObject data is streamed
/// through a single 512-byte handoff buffer; the main loop appends each packet
/// to LittleFS while the USB endpoint applies bulk NAK back-pressure.
fn service_send_object_info(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    objects: &[MtpObject],
    object_count: usize,
    info: UploadObjectInfo,
    transaction_id: u32,
    next_object_handle: &mut u32,
    pending_upload: &mut Option<(UploadObjectInfo, u32, usize)>,
) {
    if object_count >= objects.len() || pending_upload.is_some() {
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_PREFLIGHT_REJECT.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::STORE_FULL,
            transaction_id,
        );
        return;
    }

    let name = core::str::from_utf8(&info.name[..info.name_len as usize]).unwrap_or("");
    let Ok(relative_path) = PathBuf::try_from(name) else {
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_PATH_REJECT.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INVALID_PARAMETER,
            transaction_id,
        );
        return;
    };
    let final_path = path!("/").join(relative_path.as_path());
    let name_conflicts = objects[..object_count].iter().any(|object| {
        core::str::from_utf8(&object.name[..object.name_len as usize])
            .unwrap_or("")
            .eq_ignore_ascii_case(name)
    });
    if name.eq_ignore_ascii_case("MTP-README.txt")
        || name.eq_ignore_ascii_case("MTP-STATUS.txt")
        || name.eq_ignore_ascii_case(".mtp-upload.tmp")
        || name_conflicts
        || fs.exists(&final_path)
    {
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_NAME_REJECT.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::ACCESS_DENIED,
            transaction_id,
        );
        return;
    }

    let temp_path = path!("/.mtp-upload.tmp");
    // Never delete an unrecognized file at the reserved temporary path.
    // An interrupted upload is cleaned up by the main-loop abort handler.
    if fs.exists(&temp_path) {
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_TEMP_EXISTS.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::ACCESS_DENIED,
            transaction_id,
        );
        return;
    }
    let create_result = fs.open_file_with_options_and_then(
        |options| options.write(true).create_new(true),
        &temp_path,
        |file| file.sync(),
    );
    if create_result.is_err() {
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_FS_CREATE_FAIL.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::ACCESS_DENIED,
            transaction_id,
        );
        return;
    }

    xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_FS_CREATE_OK.fetch_add(1, Ordering::Relaxed);
    let handle = *next_object_handle;
    let Some(next) = handle.checked_add(1) else {
        let _ = fs.remove(&temp_path);
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::GENERAL_ERROR,
            transaction_id,
        );
        return;
    };
    *next_object_handle = next;

    *pending_upload = Some((info, handle, 0));
    loop {
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        mtp.accept_upload_info(info);
        let queued = mtp.queue_main_response_with_params(
            xcore407i_usb_mtp::mtp::response::OK,
            transaction_id,
            &[info.storage_id, info.parent, handle],
        );
        unsafe { NVIC::unmask(Interrupt::TIM2); }
        if queued { break; }
        core::hint::spin_loop();
    }
}

/// Persist a received upload to the temporary file, then atomically rename it
/// to its final name. Existing files are never truncated or replaced.
fn service_upload_data(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    data: &[u8; 512],
    length: usize,
    objects: &mut [MtpObject],
    object_count: &mut usize,
    transaction_id: u32,
    final_packet: bool,
    pending_upload: &mut Option<(UploadObjectInfo, u32, usize)>,
    staging: &mut [u8; nand_fs::PAGE_SIZE],
    staging_len: &mut usize,
) {
    let Some((info, handle, written_so_far)) = pending_upload.as_mut() else {
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::GENERAL_ERROR,
            transaction_id,
        );
        return;
    };

    let next_written = match written_so_far.checked_add(length) {
        Some(total) if total <= info.size as usize => total,
        _ => {
            *staging_len = 0;
            let _ = fs.remove(&path!("/.mtp-upload.tmp"));
            pending_upload.take();
            NVIC::mask(Interrupt::TIM2);
            let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
            mtp.finish_upload();
            unsafe { NVIC::unmask(Interrupt::TIM2); }
            queue_main_mtp_response(
                xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
                transaction_id,
            );
            return;
        }
    };

    let temp_path = path!("/.mtp-upload.tmp");
    // Pack packet payloads across NAND-page boundaries. The first MTP data
    // packet carries 12 bytes of container header, so its 500-byte payload
    // does not align with subsequent 512-byte USB packets.
    let mut data_offset = 0usize;
    let mut write_failed = false;
    while data_offset < length {
        let available = staging.len() - *staging_len;
        let chunk_len = available.min(length - data_offset);
        let next_staging_len = *staging_len + chunk_len;
        staging[*staging_len..next_staging_len]
            .copy_from_slice(&data[data_offset..data_offset + chunk_len]);
        *staging_len = next_staging_len;
        data_offset += chunk_len;

        let flush_staging = *staging_len == staging.len()
            || (final_packet && data_offset == length);
        if flush_staging {
            let flush_len = *staging_len;
            let write_result = fs.open_file_with_options_and_then(
                |options| options.write(true).append(true),
                &temp_path,
                |file| {
                    let count = file.write(&staging[..flush_len])?;
                    if count != flush_len {
                        return Err(littlefs2::io::Error::IO);
                    }
                    if final_packet && data_offset == length { file.sync()?; }
                    Ok(())
                },
            );
            if write_result.is_err() {
                write_failed = true;
                break;
            }
            *staging_len = 0;
            xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_FS_WRITE_OK
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    if write_failed {
        *staging_len = 0;
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_FS_WRITE_FAIL.fetch_add(1, Ordering::Relaxed);
        let _ = fs.remove(&temp_path);
        pending_upload.take();
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        mtp.finish_upload();
        unsafe { NVIC::unmask(Interrupt::TIM2); }
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
            transaction_id,
        );
        return;
    }

    if !final_packet {
        *written_so_far = next_written;
        return;
    }

    if next_written != info.size as usize {
        *staging_len = 0;
        let _ = fs.remove(&temp_path);
        pending_upload.take();
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        mtp.finish_upload();
        unsafe { NVIC::unmask(Interrupt::TIM2); }
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
            transaction_id,
        );
        return;
    }

    let info = *info;
    let handle = *handle;
    let name = core::str::from_utf8(&info.name[..info.name_len as usize]).unwrap_or("");
    let Ok(relative_path) = PathBuf::try_from(name) else {
        let _ = fs.remove(&temp_path);
        pending_upload.take();
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        mtp.finish_upload();
        unsafe { NVIC::unmask(Interrupt::TIM2); }
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INVALID_PARAMETER,
            transaction_id,
        );
        return;
    };
    let final_path = path!("/").join(relative_path.as_path());
    let rename_result = if !fs.exists(&final_path) {
        fs.rename(&temp_path, &final_path)
    } else {
        Err(littlefs2::io::Error::IO)
    };
    if rename_result.is_err() {
        xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_FS_RENAME_FAIL.fetch_add(1, Ordering::Relaxed);
        let _ = fs.remove(&temp_path);
        pending_upload.take();
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        mtp.finish_upload();
        unsafe { NVIC::unmask(Interrupt::TIM2); }
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
            transaction_id,
        );
        return;
    }

    xcore407i_usb_mtp::mtp::diagnostics::UPLOAD_FS_RENAME_OK.fetch_add(1, Ordering::Relaxed);
    let device_uid = read_device_uid();
    let puid = match device_uid.and_then(|uid| load_or_create_puid(fs, &final_path, &uid)) {
        Ok(puid) => puid,
        Err(_) => {
            let _ = fs.remove(&final_path);
            pending_upload.take();
            NVIC::mask(Interrupt::TIM2);
            let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
            mtp.finish_upload();
            unsafe { NVIC::unmask(Interrupt::TIM2); }
            queue_main_mtp_response(
                xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
                transaction_id,
            );
            return;
        }
    };
    let object = &mut objects[*object_count];
    object.handle = handle;
    object.name[..info.name_len as usize].copy_from_slice(&info.name[..info.name_len as usize]);
    object.name_len = info.name_len;
    object.size = info.size;
    object.is_dir = false;
    object.puid = puid;
    *object_count += 1;
    pending_upload.take();

    let capacity_bytes = fs.total_space() as u64;
    let free_bytes = fs.available_space().unwrap_or(0) as u64;
    NVIC::mask(Interrupt::TIM2);
    let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
    mtp.set_objects(&objects[..*object_count]);
    mtp.set_storage_stats(capacity_bytes, free_bytes);
    mtp.finish_upload();
    let _ = mtp.queue_main_response(
        xcore407i_usb_mtp::mtp::response::OK,
        transaction_id,
    );
    unsafe { NVIC::unmask(Interrupt::TIM2); }
}

/// Delete a host-selected regular file in main context, then publish the
/// compacted directory snapshot before acknowledging the MTP transaction.
fn service_delete_object(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    objects: &mut [MtpObject],
    object_count: &mut usize,
    handle: u32,
    transaction_id: u32,
) {
    let Some(index) = objects[..*object_count]
        .iter()
        .position(|object| object.handle == handle)
    else {
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INVALID_OBJECT_HANDLE,
            transaction_id,
        );
        return;
    };

    let object = objects[index];
    if object.is_dir {
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::OPERATION_NOT_SUPPORTED,
            transaction_id,
        );
        return;
    }
    let name = core::str::from_utf8(&object.name[..object.name_len as usize]).unwrap_or("");
    let Ok(relative_path) = PathBuf::try_from(name) else {
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::GENERAL_ERROR,
            transaction_id,
        );
        return;
    };
    let full_path = path!("/").join(relative_path.as_path());
    if fs.remove(&full_path).is_err() {
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::ACCESS_DENIED,
            transaction_id,
        );
        return;
    }

    // Preserve the handles of remaining entries; only the removed slot is
    // compacted away. Handles are not reassigned during a live session.
    for slot in index..(*object_count - 1) {
        objects[slot] = objects[slot + 1];
    }
    *object_count -= 1;
    objects[*object_count] = MtpObject::EMPTY;

    let capacity_bytes = fs.total_space() as u64;
    let free_bytes = fs.available_space().unwrap_or(0) as u64;
    NVIC::mask(Interrupt::TIM2);
    let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
    mtp.set_objects(&objects[..*object_count]);
    mtp.set_storage_stats(capacity_bytes, free_bytes);
    let _ = mtp.queue_main_response(
        xcore407i_usb_mtp::mtp::response::OK,
        transaction_id,
    );
    unsafe { NVIC::unmask(Interrupt::TIM2); }
}

/// Rename a root-level regular file and update its stable MTP handle.
fn service_rename_object(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    objects: &mut [MtpObject],
    object_count: &mut usize,
    handle: u32,
    new_name: &[u8],
    transaction_id: u32,
) {
    let Some(index) = objects[..*object_count].iter().position(|object| object.handle == handle) else {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::INVALID_OBJECT_HANDLE, transaction_id);
        return;
    };
    let object = objects[index];
    if object.is_dir || core::str::from_utf8(&object.name[..object.name_len as usize]).unwrap_or("") == "MTP-README.txt" || core::str::from_utf8(&object.name[..object.name_len as usize]).unwrap_or("") == "MTP-STATUS.txt"
        || new_name.eq_ignore_ascii_case(b".mtp-upload.tmp")
        || new_name.eq_ignore_ascii_case(b"MTP-README.txt")
        || new_name.eq_ignore_ascii_case(b"MTP-STATUS.txt")
        || new_name.is_empty() || new_name.len() > object.name.len()
    {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::ACCESS_DENIED, transaction_id);
        return;
    }
    let Ok(old_name) = core::str::from_utf8(&object.name[..object.name_len as usize]) else {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::GENERAL_ERROR, transaction_id);
        return;
    };
    let Ok(new_name_str) = core::str::from_utf8(new_name) else {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::INVALID_PARAMETER, transaction_id);
        return;
    };
    if old_name == new_name_str {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::OK, transaction_id);
        return;
    }
    if objects[..*object_count].iter().any(|entry| entry.handle != handle
        && core::str::from_utf8(&entry.name[..entry.name_len as usize]).unwrap_or("")
            .eq_ignore_ascii_case(new_name_str))
    {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::ACCESS_DENIED, transaction_id);
        return;
    }
    let (Ok(old_relative), Ok(new_relative)) = (PathBuf::try_from(old_name), PathBuf::try_from(new_name_str)) else {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::INVALID_PARAMETER, transaction_id);
        return;
    };
    let old_path = path!("/").join(old_relative.as_path());
    let new_path = path!("/").join(new_relative.as_path());
    if fs.exists(&new_path) || fs.rename(&old_path, &new_path).is_err() {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::ACCESS_DENIED, transaction_id);
        return;
    }
    objects[index].name.fill(0);
    objects[index].name[..new_name.len()].copy_from_slice(new_name);
    objects[index].name_len = new_name.len() as u8;
    NVIC::mask(Interrupt::TIM2);
    let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
    mtp.set_objects(&objects[..*object_count]);
    unsafe { NVIC::unmask(Interrupt::TIM2); }
    // Wait for the previous bulk-IN transfer to complete before queueing the
    // rename response. Dropping this response leaves Windows believing that
    // the rename did not complete (or waiting on a stale transaction).
    queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::OK, transaction_id);
}

/// Stream one root-level LittleFS file to the host. Each MTP data container is
/// emitted in USB HS-sized chunks; no whole-file buffer is used.
fn service_get_object(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    objects: &[MtpObject],
    object_count: usize,
    handle: u32,
    transaction_id: u32,
) {
    use xcore407i_usb_mtp::mtp::diagnostics as diag;
    diag::GET_OBJECT_REQUESTS.fetch_add(1, Ordering::Relaxed);
    diag::GET_OBJECT_LAST_SIZE.store(0, Ordering::Relaxed);
    diag::GET_OBJECT_BYTES_READ.store(0, Ordering::Relaxed);
    diag::GET_OBJECT_CHUNKS_QUEUED.store(0, Ordering::Relaxed);
    diag::GET_OBJECT_READ_ERRORS.store(0, Ordering::Relaxed);
    diag::GET_OBJECT_READ_US.store(0, Ordering::Relaxed);
    diag::GET_OBJECT_WAIT_US.store(0, Ordering::Relaxed);
    diag::GET_OBJECT_TOTAL_US.store(0, Ordering::Relaxed);
    let total_started = get_object_timer_us();
    let Some(object) = objects[..object_count].iter().find(|item| item.handle == handle).copied() else {
        diag::GET_OBJECT_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::INVALID_OBJECT_HANDLE, transaction_id);
        return;
    };
    diag::GET_OBJECT_LAST_SIZE.store(object.size, Ordering::Relaxed);
    if object.is_dir {
        diag::GET_OBJECT_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::OPERATION_NOT_SUPPORTED, transaction_id);
        return;
    }

    let name = core::str::from_utf8(&object.name[..object.name_len as usize]).unwrap_or("");
    let Ok(relative_path) = PathBuf::try_from(name) else {
        diag::GET_OBJECT_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::GENERAL_ERROR, transaction_id);
        return;
    };
    let full_path = path!("/").join(relative_path.as_path());

    // Small objects fit in one MTP data container. Use queue_data(), the same
    // transport path used by the other working MTP data responses.
    if object.size <= 500 {
        let mut contents = [0u8; 500];
        let mut total = 0usize;
        let read_result = fs.open_file_and_then(&full_path, |file| {
            while total < object.size as usize {
                let count = file.read(&mut contents[total..object.size as usize])?;
                if count == 0 {
                    return Err(littlefs2::io::Error::IO);
                }
                total += count;
                diag::GET_OBJECT_BYTES_READ.store(total as u32, Ordering::Relaxed);
            }
            Ok(())
        });
        if read_result.is_err() || total != object.size as usize {
            diag::GET_OBJECT_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
            queue_main_mtp_response(
                xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
                transaction_id,
            );
            return;
        }
        queue_main_mtp_object_data(transaction_id, &contents[..total]);
        diag::GET_OBJECT_CHUNKS_QUEUED.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let mut first = true;
    let mut remaining = object.size as usize;
    let result = fs.open_file_and_then(&full_path, |file| {
        // Fill the 4 KiB MTP transport buffer with one batch. The staging
        // buffer is in static SRAM, avoiding a large main-stack allocation.
        let buffer = unsafe { &mut *ptr::addr_of_mut!(GET_OBJECT_STAGING) };
        loop {
            // Keep every non-final USB packet full. The first batch reserves
            // 12 bytes in the MTP transport buffer for the container header.
            let payload_limit = if first {
                buffer.len() - xcore407i_usb_mtp::mtp::CONTAINER_HEADER_LEN
            } else {
                buffer.len()
            };
            let wanted = remaining.min(payload_limit);
            let mut count = 0usize;
            while count < wanted {
                let read_started = get_object_timer_us();
                let read = file.read(&mut buffer[count..wanted])?;
                diag::GET_OBJECT_READ_US.fetch_add(
                    get_object_timer_us().wrapping_sub(read_started),
                    Ordering::Relaxed,
                );
                if read == 0 {
                    return Err(littlefs2::io::Error::IO);
                }
                count += read;
                diag::GET_OBJECT_BYTES_READ.fetch_add(read as u32, Ordering::Relaxed);
            }
            remaining -= count;
            let last = remaining == 0;
            let wait_started = get_object_timer_us();
            wait_and_queue_object_chunk(
                transaction_id,
                object.size,
                &buffer[..count],
                first,
                last,
            );
            diag::GET_OBJECT_WAIT_US.fetch_add(
                get_object_timer_us().wrapping_sub(wait_started),
                Ordering::Relaxed,
            );
            diag::GET_OBJECT_CHUNKS_QUEUED.fetch_add(1, Ordering::Relaxed);
            first = false;
            if last {
                break;
            }
        }
        Ok(())
    });

    diag::GET_OBJECT_TOTAL_US.store(
        get_object_timer_us().wrapping_sub(total_started),
        Ordering::Relaxed,
    );

    if result.is_err() {
        diag::GET_OBJECT_READ_ERRORS.fetch_add(1, Ordering::Relaxed);
        // If a NAND read fails after a partial data container, terminate the
        // transfer with an explicit incomplete-transfer response.
        queue_main_mtp_response(
            xcore407i_usb_mtp::mtp::response::INCOMPLETE_TRANSFER,
            transaction_id,
        );
    }
}

fn wait_and_queue_object_chunk(
    transaction_id: u32,
    total_size: u32,
    bytes: &[u8],
    first: bool,
    last: bool,
) {
    loop {
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        if mtp.object_tx_ready() {
            let queued = mtp.queue_object_data_chunk(
                transaction_id,
                total_size,
                bytes,
                first,
                last,
            );
            unsafe { NVIC::unmask(Interrupt::TIM2); }
            if queued {
                break;
            }
        } else {
            unsafe { NVIC::unmask(Interrupt::TIM2); }
        }
        core::hint::spin_loop();
    }
}

fn queue_main_mtp_object_data(transaction_id: u32, payload: &[u8]) {
    loop {
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        if mtp.object_tx_ready() {
            let queued = mtp.queue_object_data(transaction_id, payload);
            unsafe { NVIC::unmask(Interrupt::TIM2); }
            if queued {
                break;
            }
        } else {
            unsafe { NVIC::unmask(Interrupt::TIM2); }
        }
        core::hint::spin_loop();
    }
}

fn queue_main_mtp_response(code: u16, transaction_id: u32) {
    loop {
        NVIC::mask(Interrupt::TIM2);
        let mtp = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) };
        if mtp.object_tx_ready() {
            let _ = mtp.queue_main_response(code, transaction_id);
            unsafe { NVIC::unmask(Interrupt::TIM2); }
            break;
        }
        unsafe { NVIC::unmask(Interrupt::TIM2); }
        core::hint::spin_loop();
    }
}

/// Read the free-running TIM5 counter configured at 1 MHz.
#[inline]
fn get_object_timer_us() -> u32 {
    unsafe { (*pac::TIM5::ptr()).cnt().read().bits() }
}

/// TIM5 is independent of TIM2, which remains dedicated to USB polling.
/// APB1 timer clock is 84 MHz; prescaler 83 gives a 1 MHz, 32-bit counter.
fn configure_get_object_timer(rcc: &pac::RCC, tim5: &pac::TIM5) {
    rcc.apb1enr().modify(|_, w| w.tim5en().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim5rst().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim5rst().clear_bit());

    tim5.cr1().write(|w| w.cen().clear_bit());
    tim5.dier().write(|w| w.uie().clear_bit());
    tim5.psc().write(|w| unsafe { w.psc().bits(83) });
    tim5.arr().write(|w| unsafe { w.arr().bits(u32::MAX) });
    tim5.egr().write(|w| w.ug().set_bit());
    tim5.sr().write(|w| w.uif().clear_bit());
    tim5.cnt().write(|w| unsafe { w.cnt().bits(0) });
    tim5.cr1().write(|w| w.cen().set_bit());
}

/// Poll the USB HS device at 8 kHz (one call per 125 us microframe).
#[interrupt]
fn TIM2() {
    let tim2 = unsafe { &*pac::TIM2::ptr() };
    tim2.sr().write(|w| w.uif().clear_bit());

    let usb_ptr = USB_DEVICE_PTR.load(Ordering::Acquire);
    let mtp_ptr = MTP_PTR.load(Ordering::Acquire);
    if usb_ptr.is_null() || mtp_ptr.is_null() {
        return;
    }

    // SAFETY: the pointers are initialized before TIM2 is enabled. Main only
    // mutates MTP with TIM2 masked; all other USB access belongs to this ISR.
    let usb = unsafe { &mut *usb_ptr };
    let mtp = unsafe { &mut *mtp_ptr };
    let _ = usb.poll(&mut [mtp]);
}

fn configure_usb_poll_timer(rcc: &pac::RCC, tim2: &pac::TIM2, nvic: &mut NVIC) {
    // APB1 timer clock is 84 MHz with SYSCLK=168 MHz and APB1 prescaler /4.
    // PSC=0, ARR=10499 => 8 kHz / 125 us.
    let timer_ticks = 84_000_000 / USB_POLL_HZ;

    rcc.apb1enr().modify(|_, w| w.tim2en().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim2rst().set_bit());
    rcc.apb1rstr().modify(|_, w| w.tim2rst().clear_bit());

    tim2.cr1().write(|w| w.cen().clear_bit());
    tim2.dier().write(|w| w.uie().clear_bit());
    tim2.psc().write(|w| unsafe { w.psc().bits(0) });
    tim2.arr().write(|w| unsafe { w.arr().bits(timer_ticks - 1) });
    tim2.egr().write(|w| w.ug().set_bit());
    tim2.sr().write(|w| w.uif().clear_bit());

    unsafe {
        nvic.set_priority(Interrupt::TIM2, TIM2_PRIORITY << 4);
        NVIC::unpend(interrupt::TIM2);
    }

    tim2.dier().write(|w| w.uie().set_bit());
    tim2.cr1().write(|w| w.cen().set_bit());
    unsafe { NVIC::unmask(interrupt::TIM2); }
}
