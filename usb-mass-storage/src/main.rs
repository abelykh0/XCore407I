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
use stm32f4::stm32f407::{interrupt, Interrupt};
use stm32f4xx_hal::pac;
use synopsys_usb_otg::UsbBus;
use usb_device::descriptor::lang_id::LangID;
use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};

use xcore407i_usb_mtp::{
    board,
    bus::usb_bus,
    mtp::{MtpObject, MtpUsbClass},
    nand_fs,
    usb_hs::F407UsbHs,
};

const USB_POLL_HZ: u32 = 8_000;
const TIM2_PRIORITY: u8 = 2;

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
    configure_usb_poll_timer(&rcc, &peripherals.TIM2, &mut core.NVIC);

    // Mount the existing volume once. The report is generated from this
    // mount, then the same mounted filesystem remains alive while the main
    // context services future MTP requests. No NAND/LittleFS work runs in TIM2.
    let mounted = nand_fs::with_mounted_report::<()>(|fs, report, report_len| {
        // Seed a few real files into the existing LittleFS volume. Each file
        // is created only when absent, so normal boots never overwrite user
        // data. These writes happen in main context, never in TIM2.
        seed_test_files(fs)?;

        // Build a bounded snapshot of the LittleFS root directory in main
        // context. Directory entries are copied out; no LittleFS object is
        // accessed by the USB polling interrupt.
        let capacity_bytes = fs.total_space() as u64;
        let free_bytes = fs.available_space().unwrap_or(0) as u64;
        let mut objects = [MtpObject::EMPTY; 16];
        let mut object_count = 0usize;
        fs.read_dir_and_then(path!("/"), |entries| {
            for entry in entries.skip(2) {
                let entry = entry?;
                if object_count == objects.len() {
                    break;
                }
                let name = entry.file_name().as_ref().as_bytes();
                let name_len = name.len().min(objects[object_count].name.len());
                objects[object_count].name[..name_len].copy_from_slice(&name[..name_len]);
                objects[object_count].name_len = name_len as u8;
                objects[object_count].handle = object_count as u32 + 1;
                let metadata = entry.metadata();
                objects[object_count].size = metadata.len().min(u32::MAX as usize) as u32;
                objects[object_count].is_dir = metadata.is_dir();
                object_count += 1;
            }
            Ok(())
        })?;

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

        loop {
            // Pull commands out of the USB ISR and perform all LittleFS work
            // here, with TIM2 free to keep servicing USB HS.
            NVIC::mask(Interrupt::TIM2);
            let request = unsafe { &mut *MTP_PTR.load(Ordering::Acquire) }
                .take_get_object_request();
            peripherals.TIM2.sr().write(|w| w.uif().clear_bit());
            unsafe {
                NVIC::unpend(Interrupt::TIM2);
                NVIC::unmask(Interrupt::TIM2);
            }

            if let Some((handle, transaction_id)) = request {
                service_get_object(fs, &objects, object_count, handle, transaction_id);
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
    if !fs.exists(path!("/EMPTY.TXT")) {
        fs.write(path!("/EMPTY.TXT"), &[])?;
    }
    Ok(())
}

/// Stream one root-level LittleFS file to the host. Each MTP data container is
/// emitted in bounded 244-byte payload chunks; no whole-file buffer is used.
fn service_get_object(
    fs: &littlefs2::fs::Filesystem<'_, nand_fs::NandStorage>,
    objects: &[MtpObject; 16],
    object_count: usize,
    handle: u32,
    transaction_id: u32,
) {
    let Some(object) = objects[..object_count].iter().find(|item| item.handle == handle).copied() else {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::INVALID_OBJECT_HANDLE, transaction_id);
        return;
    };
    if object.is_dir {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::OPERATION_NOT_SUPPORTED, transaction_id);
        return;
    }

    let name = core::str::from_utf8(&object.name[..object.name_len as usize]).unwrap_or("");
    let Ok(relative_path) = PathBuf::try_from(name) else {
        queue_main_mtp_response(xcore407i_usb_mtp::mtp::response::GENERAL_ERROR, transaction_id);
        return;
    };
    let full_path = path!("/").join(relative_path.as_path());
    let mut first = true;
    let mut remaining = object.size as usize;
    let result = fs.open_file_and_then(&full_path, |file| {
        let mut buffer = [0u8; 500];
        loop {
            let count = if remaining == 0 {
                0
            } else {
                let read_len = remaining.min(buffer.len());
                file.read(&mut buffer[..read_len])?
            };
            if remaining != 0 && count == 0 {
                return Err(littlefs2::io::Error::IO);
            }
            remaining = remaining.saturating_sub(count);
            let last = remaining == 0;
            wait_and_queue_object_chunk(
                transaction_id,
                object.size,
                &buffer[..count],
                first,
                last,
            );
            first = false;
            if last {
                break;
            }
        }
        Ok(())
    });

    if result.is_err() {
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
