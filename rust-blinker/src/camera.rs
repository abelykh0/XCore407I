//! C-style compact color canvas and NV12 UVC packetizer.
//!
//! This mirrors the original C canvas: 128 KiB of source data split between
//! ordinary SRAM and CCM, with two 768-byte row caches in CCM. Color stores
//! packed Cb/Cr pairs at 256x256; BW stores Y at 512x256.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[cfg(not(feature = "camera-bw"))]
use crate::palette::{UV_PALETTE, Y_HASH_TABLE};

#[cfg(not(feature = "camera-bw"))]
const fn make_duplicated_y_table() -> [u16; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < table.len() {
        let y = Y_HASH_TABLE[i] as u16;
        table[i] = y | (y << 8);
        i += 1;
    }
    table
}

#[cfg(not(feature = "camera-bw"))]
const DUPLICATED_Y_TABLE: [u16; 256] = make_duplicated_y_table();

pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 512;
pub const Y_PLANE_BYTES: usize = WIDTH * HEIGHT;
pub const FRAME_BYTES: usize = Y_PLANE_BYTES + (WIDTH * HEIGHT / 2);
pub const UVC_HEADER_BYTES: usize = 4;
// Match the original C packetizer: one 512-pixel NV12 row's worth of payload
// (512 Y bytes + 256 UV bytes) plus the four-byte UVC header.
pub const PAYLOAD_BYTES: usize = WIDTH * 3 / 2;
pub const TRANSFER_BYTES: usize = UVC_HEADER_BYTES + PAYLOAD_BYTES;

#[cfg(feature = "camera-bw")]
pub const CANVAS_WIDTH: usize = WIDTH;
#[cfg(not(feature = "camera-bw"))]
pub const CANVAS_WIDTH: usize = WIDTH / 2;
pub const CANVAS_HEIGHT: usize = HEIGHT / 2;
pub const CANVAS_BYTES: usize = WIDTH * CANVAS_HEIGHT;
pub const CANVAS_TAIL_BYTES: usize = 19_200;
pub const CANVAS_MAIN_BYTES: usize = CANVAS_BYTES - CANVAS_TAIL_BYTES;

#[no_mangle]
pub static UVC_DEBUG_RENDER_Y_CYCLES: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static UVC_DEBUG_RENDER_UV_CYCLES: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static UVC_DEBUG_RENDER_COPY_CYCLES: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static UVC_DEBUG_RENDER_OVERLAY_CYCLES: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
fn cycle_counter() -> u32 {
    unsafe { (0xE000_1004 as *const u32).read_volatile() }
}

#[inline(always)]
fn record_render_cycles(start: u32, counter: &AtomicU32) {
    counter.fetch_add(cycle_counter().wrapping_sub(start), Ordering::Relaxed);
}

#[repr(C, align(4))]
struct AlignedCanvas<const N: usize>([u8; N]);

static mut DEMO_CANVAS: AlignedCanvas<CANVAS_MAIN_BYTES> = AlignedCanvas([0; CANVAS_MAIN_BYTES]);
#[link_section = ".ccmram.canvas"]
static mut DEMO_CANVAS_TAIL: AlignedCanvas<CANVAS_TAIL_BYTES> =
    AlignedCanvas([0; CANVAS_TAIL_BYTES]);

#[repr(C, align(4))]
struct RowBuffers([u8; WIDTH * 2]);
#[link_section = ".ccmram.rows"]
static mut ROW_BUFFERS: RowBuffers = RowBuffers([0; WIDTH * 2]);
static CAMERA_IN_USE: AtomicBool = AtomicBool::new(false);
static CLOCK_VERSION: AtomicU32 = AtomicU32::new(0);

fn write_canvas(index: usize, value: u8) {
    unsafe {
        if index < CANVAS_MAIN_BYTES {
            core::ptr::addr_of_mut!(DEMO_CANVAS.0).cast::<u8>().add(index).write(value);
        } else {
            core::ptr::addr_of_mut!(DEMO_CANVAS_TAIL.0).cast::<u8>()
                .add(index - CANVAS_MAIN_BYTES).write(value);
        }
    }
}

fn canvas_read_chunk(offset: usize) -> (*const u8, usize) {
    unsafe {
        if offset < CANVAS_MAIN_BYTES {
            (
                core::ptr::addr_of!(DEMO_CANVAS.0).cast::<u8>().add(offset),
                CANVAS_MAIN_BYTES - offset,
            )
        } else {
            let tail_offset = offset - CANVAS_MAIN_BYTES;
            (
                core::ptr::addr_of!(DEMO_CANVAS_TAIL.0)
                    .cast::<u8>()
                    .add(tail_offset),
                CANVAS_TAIL_BYTES - tail_offset,
            )
        }
    }
}

/// Initialize the demo chart in the compact canvas before USB streaming.
pub fn initialize_demo_canvas() {
    #[cfg(not(feature = "camera-bw"))]
    initialize_demo_colors_canvas();

    #[cfg(feature = "camera-bw")]
    {
    let mut y = 0;
    while y < CANVAS_HEIGHT {
        let mut x = 0;
        while x < CANVAS_WIDTH {
            #[cfg(not(feature = "camera-bw"))]
            let mut color = ((y / 32) * 8 + x / 32) as u8;
            #[cfg(feature = "camera-bw")]
            let mut color = if ((x / 32) + (y / 16)) & 1 == 0 { 255 } else { 0 };

            #[cfg(not(feature = "camera-bw"))]
            {
                // 8x8 color chart, matching the original C palette demo.

                // Camera silhouette and lens make this a recognizable test image,
                // not just a set of strips.
                if x >= 88 && x < 168 && y >= 96 && y < 160 {
                    color = 21; // gray camera body
                }
                if x >= 108 && x < 148 && y >= 80 && y < 96 {
                    color = 21; // viewfinder
                }
                let dx = x as i32 - 128;
                let dy = y as i32 - 128;
                let radius_sq = dx * dx + dy * dy;
                if radius_sq <= 22 * 22 && x >= 88 && x < 168 && y >= 96 && y < 160 {
                    color = if radius_sq > 17 * 17 { 63 } else { 48 };
                }
            }
            #[cfg(feature = "camera-bw")]
            {
                // High-contrast camera target drawn directly into luma bytes.
                if x >= 72 && x < 184 && y >= 72 && y < 176 {
                    color = 219;
                }
                if x >= 96 && x < 160 && y >= 56 && y < 72 {
                    color = 219;
                }
                let dx = x as i32 - 128;
                let dy = y as i32 - 124;
                let r2 = dx * dx + dy * dy;
                if r2 <= 24 * 24 && x >= 72 && x < 184 && y >= 72 && y < 176 {
                    color = if r2 > 24 * 24 { 255 } else { 0 };
                }
            }
            #[cfg(feature = "camera-bw")]
            let value = GRAYSCALE[color as usize & 0x3f];
            #[cfg(not(feature = "camera-bw"))]
            let value = color;
            #[cfg(feature = "camera-bw")]
            write_canvas(y * CANVAS_WIDTH + x, value);
            #[cfg(not(feature = "camera-bw"))]
            {
                let uv = UV_PALETTE[value as usize & 0x3f];
                let offset = (y * CANVAS_WIDTH + x) * 2;
                write_canvas(offset, uv[0]);
                write_canvas(offset + 1, uv[1]);
            }
            x += 1;
        }
        y += 1;
    }
    }
}

#[cfg(not(feature = "camera-bw"))]
fn initialize_demo_colors_canvas() {
    const GREY_ON_BLACK: u16 = 0x2A10;

    let mut y = 0;
    while y < CANVAS_HEIGHT {
        let mut x = 0;
        while x < CANVAS_WIDTH {
            set_demo_pixel(x, y, GREY_ON_BLACK as u8);
            x += 1;
        }
        y += 1;
    }

    draw_demo_char(0, 0, 0xC9, GREY_ON_BLACK);
    draw_demo_char(31, 0, 0xBB, GREY_ON_BLACK);
    draw_demo_char(0, 31, 0xC8, GREY_ON_BLACK);
    draw_demo_char(31, 31, 0xBC, GREY_ON_BLACK);
    let mut x = 1;
    while x < 31 {
        draw_demo_char(x, 0, 0xCD, GREY_ON_BLACK);
        draw_demo_char(x, 31, 0xCD, GREY_ON_BLACK);
        x += 1;
    }
    let mut y = 1;
    while y < 31 {
        draw_demo_char(0, y, 0xBA, GREY_ON_BLACK);
        draw_demo_char(31, y, 0xBA, GREY_ON_BLACK);
        y += 1;
    }

    draw_demo_colors(3, 2, 4, 4, 4);
    draw_demo_colors(10, 2, 4, 0, 0);
    draw_demo_colors(17, 2, 0, 4, 0);
    draw_demo_colors(24, 2, 0, 0, 4);
    draw_demo_colors(10, 6, 0, 4, 4);
    draw_demo_colors(17, 6, 4, 4, 0);
    draw_demo_colors(24, 6, 4, 0, 4);

    draw_demo_colors(3, 10, 0, 1, 2);
    draw_demo_colors(3, 11, 0, 1, 3);
    draw_demo_colors(3, 12, 0, 2, 1);
    draw_demo_colors(3, 13, 0, 2, 3);
    draw_demo_colors(3, 14, 0, 3, 1);
    draw_demo_colors(3, 15, 0, 3, 2);
    draw_demo_colors(3, 16, 1, 0, 2);
    draw_demo_colors(3, 17, 1, 0, 3);
    draw_demo_colors(3, 18, 1, 1, 2);
    draw_demo_colors(3, 19, 1, 1, 3);
    draw_demo_colors(3, 20, 1, 2, 0);
    draw_demo_colors(3, 21, 1, 2, 1);

    draw_demo_colors(10, 12, 1, 2, 2);
    draw_demo_colors(10, 13, 1, 2, 3);
    draw_demo_colors(10, 14, 1, 3, 0);
    draw_demo_colors(10, 15, 1, 3, 1);
    draw_demo_colors(10, 16, 1, 3, 2);
    draw_demo_colors(10, 17, 1, 3, 3);
    draw_demo_colors(10, 18, 2, 0, 1);
    draw_demo_colors(10, 19, 2, 0, 3);
    draw_demo_colors(10, 20, 2, 1, 0);
    draw_demo_colors(10, 21, 2, 1, 1);

    draw_demo_colors(17, 12, 2, 1, 2);
    draw_demo_colors(17, 13, 2, 1, 3);
    draw_demo_colors(17, 14, 2, 2, 1);
    draw_demo_colors(17, 15, 2, 2, 3);
    draw_demo_colors(17, 16, 2, 3, 0);
    draw_demo_colors(17, 17, 2, 3, 1);
    draw_demo_colors(17, 18, 2, 3, 2);
    draw_demo_colors(17, 19, 2, 3, 3);
    draw_demo_colors(17, 20, 3, 0, 1);
    draw_demo_colors(17, 21, 3, 0, 2);

    draw_demo_colors(24, 12, 3, 1, 0);
    draw_demo_colors(24, 13, 3, 1, 1);
    draw_demo_colors(24, 14, 3, 1, 2);
    draw_demo_colors(24, 15, 3, 1, 3);
    draw_demo_colors(24, 16, 3, 2, 0);
    draw_demo_colors(24, 17, 3, 2, 1);
    draw_demo_colors(24, 18, 3, 2, 2);
    draw_demo_colors(24, 19, 3, 2, 3);
    draw_demo_colors(24, 20, 3, 3, 1);
    draw_demo_colors(24, 21, 3, 3, 2);
}

#[cfg(not(feature = "camera-bw"))]
fn draw_demo_char(cell_x: usize, cell_y: usize, character: u8, attribute: u16) {
    use crate::font8x8::FONT8X8;

    let glyph = (character as usize - 1) * 8;
    let foreground = (attribute >> 8) as u8 & 0x3f;
    let background = attribute as u8 & 0x3f;
    let mut row = 0;
    while row < 8 {
        let bits = FONT8X8[glyph + row];
        let mut column = 0;
        while column < 8 {
            let color = if bits & (0x80 >> column) != 0 { foreground } else { background };
            set_demo_pixel(cell_x * 8 + column, cell_y * 8 + row, color);
            column += 1;
        }
        row += 1;
    }
}

#[cfg(not(feature = "camera-bw"))]
fn draw_demo_text(cell_x: usize, cell_y: usize, text: &[u8], attribute: u16) {
    for (i, character) in text.iter().enumerate() {
        draw_demo_char(cell_x + i, cell_y, *character, attribute);
    }
}

#[cfg(not(feature = "camera-bw"))]
fn draw_demo_colors(x: usize, y: usize, red: u8, green: u8, blue: u8) {
    if red < 4 && green < 4 && blue < 4 {
        let color = (blue << 4) | (green << 2) | red;
        draw_demo_text(x, y, &[0xDB, 0xDB, 0xDD], (color as u16) << 8);
        draw_demo_color_label(x + 3, y, color);
        return;
    }

    let grayscale = red == 4 && green == 4 && blue == 4;
    let mut variable = 1;
    let mut row = y;
    while variable < 4 {
        let r = if red < 4 { red } else { variable };
        let g = if green < 4 { green } else { variable };
        let b = if blue < 4 { blue } else { variable };
        let color = (b << 4) | (g << 2) | r;
        if grayscale || (color & 0x03) * 0x15 != color {
            draw_demo_text(x, row, &[0xDB, 0xDB, 0xDD], (color as u16) << 8);
            draw_demo_color_label(x + 3, row, color);
            row += 1;
        }
        variable += 1;
    }
}

#[cfg(not(feature = "camera-bw"))]
fn draw_demo_color_label(x: usize, y: usize, color: u8) {
    let value = ((color & 0x03) * 100) + (((color >> 2) & 0x03) * 10) + ((color >> 4) & 0x03);
    let digits = [b'0' + value / 100, b'0' + (value / 10) % 10, b'0' + value % 10];
    draw_demo_text(x, y, &digits, 0x2A10);
}

/// Change one logical color pixel. Every source pixel expands to a 2x2 block
/// in the 512x512 NV12 stream, as in the original C color mode.
pub fn set_demo_pixel(x: usize, y: usize, color: u8) {
    if x < CANVAS_WIDTH && y < CANVAS_HEIGHT {
        #[cfg(feature = "camera-bw")]
        write_canvas(y * CANVAS_WIDTH + x, GRAYSCALE[(color & 0x3f) as usize]);
        #[cfg(not(feature = "camera-bw"))]
        {
            let uv = UV_PALETTE[(color & 0x3f) as usize];
            let offset = (y * CANVAS_WIDTH + x) * 2;
            write_canvas(offset, uv[0]);
            write_canvas(offset + 1, uv[1]);
        }
    }
}

/// Update the top-center clock text directly in the source canvas.
pub fn update_clock_from_rtc() {
    let text = crate::rtc::display_time();
    const CLOCK_ATTRIBUTE: u16 = 0x2a10;

    #[cfg(not(feature = "camera-bw"))]
    {
        const CLOCK_CELL_X: usize = (CANVAS_WIDTH - 10 * 8) / 2 / 8;
        draw_demo_text(CLOCK_CELL_X, 0, &text, CLOCK_ATTRIBUTE);
    }

    #[cfg(feature = "camera-bw")]
    draw_clock_text_bw(&text, CLOCK_ATTRIBUTE);

    CLOCK_VERSION.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "camera-bw")]
fn draw_clock_text_bw(text: &[u8; 10], attribute: u16) {
    use crate::font8x8::FONT8X8;

    let left = (WIDTH - 10 * 16) / 2;
    let foreground = (attribute >> 8) as u8 & 0x3f;
    let background = attribute as u8 & 0x3f;
    for (character_index, character) in text.iter().enumerate() {
        let glyph = (*character as usize - 1) * 8;
        for row in 0..8 {
            let bits = FONT8X8[glyph + row];
            for column in 0..8 {
                let color = if bits & (0x80 >> column) != 0 {
                    foreground
                } else {
                    background
                };
                let x = left + character_index * 16 + column * 2;
                let y = row * 2;
                set_demo_pixel(x, y, color);
                set_demo_pixel(x + 1, y, color);
                set_demo_pixel(x, y + 1, color);
                set_demo_pixel(x + 1, y + 1, color);
            }
        }
    }
}

#[cfg(feature = "camera-bw")]
const GRAYSCALE: [u8; 64] = [
    0,6,12,18,61,67,73,79,122,128,134,140,183,189,195,201,
    18,24,30,36,79,85,91,97,140,146,152,158,201,207,213,219,
    36,42,48,54,97,103,109,115,158,164,170,176,219,225,231,237,
    54,60,66,72,115,121,127,133,176,182,188,194,237,243,249,255,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Packet {
    pub len: usize,
    pub frame_complete: bool,
}

/// Emits the same compact demo image repeatedly as UVC payloads.
pub(crate) struct Nv12DemoImage {
    offset: usize,
    frame_id: bool,
    cpu_available_percent: u8,
    cpu_generate_percent: u8,
    cpu_submit_percent: u8,
    cached_y_source_row: u16,
    cached_uv_source_row: u16,
    cached_clock_version: u32,
}

impl Nv12DemoImage {
    /// Claim the shared row buffers for the single live camera instance.
    /// Returns `None` while another camera instance owns them.
    pub(crate) fn try_new() -> Option<Self> {
        CAMERA_IN_USE
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        Some(Self {
            offset: 0,
            frame_id: false,
            cpu_available_percent: 100,
            cpu_generate_percent: 0,
            cpu_submit_percent: 0,
            cached_y_source_row: u16::MAX,
            cached_uv_source_row: u16::MAX,
            cached_clock_version: CLOCK_VERSION.load(Ordering::Relaxed),
        })
    }

    pub(crate) fn reset(&mut self) {
        self.offset = 0;
        self.frame_id = false;
        self.cpu_available_percent = 100;
        self.cpu_generate_percent = 0;
        self.cpu_submit_percent = 0;
        self.cached_y_source_row = u16::MAX;
        self.cached_uv_source_row = u16::MAX;
        self.cached_clock_version = CLOCK_VERSION.load(Ordering::Relaxed);
    }

    /// Set the estimated CPU percentage available outside measured UVC work.
    pub fn set_cpu_metrics(&mut self, available: u8, generate: u8, submit: u8) {
        self.cpu_available_percent = available.min(100);
        self.cpu_generate_percent = generate.min(100);
        self.cpu_submit_percent = submit.min(100);
    }

    /// Fill one UVC payload transfer into caller-owned storage.
    ///
    /// `out` must hold at least `TRANSFER_BYTES` bytes. Bytes 0..4 are the
    /// UVC payload header; the remaining bytes are a chunk of the NV12 frame.
    pub fn next_packet(&mut self, out: &mut [u8]) -> Option<Packet> {
        self.next_payload_transfer(out, TRANSFER_BYTES)
    }

    /// Fill one UVC payload transfer, which may span multiple USB packets in
    /// a high-bandwidth HS microframe. UVC places a single payload header at
    /// the start of the transfer; continuation transactions contain video
    /// bytes only.
    pub fn next_payload_transfer(
        &mut self,
        out: &mut [u8],
        max_transfer_bytes: usize,
    ) -> Option<Packet> {
        if out.len() < UVC_HEADER_BYTES {
            return None;
        }

        let transfer_capacity = max_transfer_bytes.min(out.len());
        if transfer_capacity < UVC_HEADER_BYTES {
            return None;
        }
        let remaining = FRAME_BYTES - self.offset;
        let data_len = remaining.min(transfer_capacity - UVC_HEADER_BYTES);
        if data_len == 0 {
            return None;
        }

        let complete = data_len == remaining;
        out[0] = UVC_HEADER_BYTES as u8;
        // EOH terminates this four-byte header; FID toggles per frame and
        // EOF marks the last payload of each frame.
        out[1] = 0x80 | u8::from(self.frame_id) | if complete { 0x02 } else { 0 };
        out[2] = 0; // PTS not present
        out[3] = 0; // SCR not present
        self.fill_nv12(
            self.offset,
            &mut out[UVC_HEADER_BYTES..UVC_HEADER_BYTES + data_len],
        );
        /* CPU diagnostics are retained here for later use, but hidden from
         * the demo image so its contents remain unobstructed.
        let stage_start = cycle_counter();
        overlay_cpu_text(
            self.offset,
            &mut out[UVC_HEADER_BYTES..UVC_HEADER_BYTES + data_len],
            self.cpu_available_percent,
            self.cpu_generate_percent,
            self.cpu_submit_percent,
        );
        record_render_cycles(stage_start, &UVC_DEBUG_RENDER_OVERLAY_CYCLES);
        */

        self.offset += data_len;
        if complete {
            self.offset = 0;
            self.frame_id = !self.frame_id;
        }

        Some(Packet {
            len: UVC_HEADER_BYTES + data_len,
            frame_complete: complete,
        })
    }
}

impl Drop for Nv12DemoImage {
    fn drop(&mut self) {
        CAMERA_IN_USE.store(false, Ordering::Release);
    }
}

impl Nv12DemoImage {
    /// Render sequential NV12 bytes from the mode-specific compact canvas.
    /// Color doubles both axes from 256x256 UV pairs; BW doubles the 512x256
    /// luma canvas vertically and uses neutral chroma.
    fn fill_nv12(&mut self, offset: usize, out: &mut [u8]) {
        let mut pos = offset;
        let mut written = 0;
        while written < out.len() && pos < FRAME_BYTES {
            if pos < Y_PLANE_BYTES {
                let output_row = pos / WIDTH;
                let x = pos % WIDTH;
                let source_row = (output_row / 2) as u16;
                let stage_start = cycle_counter();
                self.prepare_y_row(source_row);
                record_render_cycles(stage_start, &UVC_DEBUG_RENDER_Y_CYCLES);
                let run = (WIDTH - x)
                    .min(Y_PLANE_BYTES - pos)
                    .min(out.len() - written);
                unsafe {
                    let stage_start = cycle_counter();
                    crate::copy_words::copy_words(
                        y_row_ptr().add(x), out.as_mut_ptr().add(written), run,
                    );
                    record_render_cycles(stage_start, &UVC_DEBUG_RENDER_COPY_CYCLES);
                }
                pos += run;
                written += run;
            } else {
                let chroma_pos = pos - Y_PLANE_BYTES;
                let x = chroma_pos % WIDTH;
                let run = (WIDTH - x).min(FRAME_BYTES - pos).min(out.len() - written);
                #[cfg(feature = "camera-bw")]
                {
                    let source_row = (chroma_pos / WIDTH) as u16;
                    let stage_start = cycle_counter();
                    self.prepare_uv_row(source_row);
                    record_render_cycles(stage_start, &UVC_DEBUG_RENDER_UV_CYCLES);
                    unsafe {
                        let stage_start = cycle_counter();
                        crate::copy_words::copy_words(
                            uv_row_ptr().add(x), out.as_mut_ptr().add(written), run,
                        );
                        record_render_cycles(stage_start, &UVC_DEBUG_RENDER_COPY_CYCLES);
                    }
                }
                #[cfg(not(feature = "camera-bw"))]
                unsafe {
                    // Color canvas chroma is already packed in NV12 row order.
                    let stage_start = cycle_counter();
                    copy_canvas_range(chroma_pos, out.as_mut_ptr().add(written), run);
                    record_render_cycles(stage_start, &UVC_DEBUG_RENDER_COPY_CYCLES);
                }
                pos += run;
                written += run;
            }
        }
        out[written..].fill(0);
    }

    fn prepare_y_row(&mut self, source_row: u16) {
        if source_row == 0 {
            let clock_version = CLOCK_VERSION.load(Ordering::Relaxed);
            if self.cached_clock_version != clock_version {
                self.cached_clock_version = clock_version;
                self.cached_y_source_row = u16::MAX;
            }
        }
        if self.cached_y_source_row == source_row {
            return;
        }
        #[cfg(feature = "camera-bw")]
        let source_bytes_per_pixel = 1;
        #[cfg(not(feature = "camera-bw"))]
        let source_bytes_per_pixel = 2;

        let row_offset = source_row as usize * CANVAS_WIDTH * source_bytes_per_pixel;
        let mut pixel_x = 0;
        while pixel_x < CANVAS_WIDTH {
            let byte_offset = row_offset + pixel_x * source_bytes_per_pixel;
            let (source, available_bytes) = canvas_read_chunk(byte_offset);
            let chunk_pixels = (CANVAS_WIDTH - pixel_x)
                .min(available_bytes / source_bytes_per_pixel);
            let mut i = 0;
            #[cfg(feature = "camera-bw")]
            {
                // BW canvas already has the output width; only rows are doubled.
                let destination = unsafe { y_row_ptr().add(pixel_x) };
                while i < chunk_pixels {
                    let y = unsafe { source.add(i).read() };
                    unsafe { destination.add(i).write(y) };
                    i += 1;
                }
            }
            #[cfg(not(feature = "camera-bw"))]
            {
                // Canvas chunk starts and each color pixel are 16-bit aligned.
                let source_pairs = source.cast::<u16>();
                let destination = unsafe { y_row_ptr().cast::<u16>().add(pixel_x) };
                while i < chunk_pixels {
                    let pair = unsafe { source_pairs.add(i).read() };
                    let hash = pair as u8 ^ (pair >> 8) as u8;
                    unsafe { destination.add(i).write(DUPLICATED_Y_TABLE[hash as usize]) };
                    i += 1;
                }
            }
            pixel_x += chunk_pixels;
        }
        self.cached_y_source_row = source_row;
    }

    #[cfg(feature = "camera-bw")]
    fn prepare_uv_row(&mut self, source_row: u16) {
        if self.cached_uv_source_row == source_row {
            return;
        }
        #[cfg(feature = "camera-bw")]
        unsafe { uv_row_ptr().write_bytes(128, WIDTH) };
        #[cfg(not(feature = "camera-bw"))]
        unsafe {
            copy_canvas_range(
                source_row as usize * WIDTH,
                uv_row_ptr(),
                WIDTH,
            );
        }
        self.cached_uv_source_row = source_row;
    }
}

#[inline(always)]
unsafe fn y_row_ptr() -> *mut u8 {
    core::ptr::addr_of_mut!(ROW_BUFFERS.0).cast::<u8>()
}

#[cfg(feature = "camera-bw")]
#[inline(always)]
unsafe fn uv_row_ptr() -> *mut u8 {
    core::ptr::addr_of_mut!(ROW_BUFFERS.0).cast::<u8>().add(WIDTH)
}

/// Copy an aligned region from the C-style main SRAM + CCM canvas split.
#[cfg(not(feature = "camera-bw"))]
unsafe fn copy_canvas_range(mut offset: usize, mut destination: *mut u8, mut bytes: usize) {
    while bytes != 0 {
        let (source, available) = if offset < CANVAS_MAIN_BYTES {
            (
                core::ptr::addr_of!(DEMO_CANVAS.0).cast::<u8>().add(offset),
                CANVAS_MAIN_BYTES - offset,
            )
        } else {
            let tail_offset = offset - CANVAS_MAIN_BYTES;
            (
                core::ptr::addr_of!(DEMO_CANVAS_TAIL.0).cast::<u8>().add(tail_offset),
                CANVAS_TAIL_BYTES - tail_offset,
            )
        };
        let run = bytes.min(available);
        crate::copy_words::copy_words(source, destination, run);
        offset += run;
        destination = destination.add(run);
        bytes -= run;
    }
}

const OVERLAY_X: usize = 8;
const OVERLAY_Y: usize = 8;
const OVERLAY_TEXT: &[u8] = b"UVC FREE: ";
const OVERLAY_LINE_2_PREFIX: &[u8] = b"GEN: ";
const OVERLAY_LINE_2_MIDDLE: &[u8] = b" TX: ";
const OVERLAY_CHARS: usize = OVERLAY_LINE_2_PREFIX.len() + 4 + OVERLAY_LINE_2_MIDDLE.len() + 4;

fn overlay_cpu_text(offset: usize, out: &mut [u8], available: u8, generate: u8, submit: u8) {
    use crate::font8x8::FONT8X8;

    let end = offset + out.len();
    for y in OVERLAY_Y..OVERLAY_Y + 16 {
        let row_start = y * WIDTH + OVERLAY_X;
        let row_end = row_start + OVERLAY_CHARS * 8;
        let start = offset.max(row_start);
        let stop = end.min(row_end);
        for pos in start..stop {
            let x = pos - row_start;
            let text_col = x / 8;
            let line = (y - OVERLAY_Y) / 8;
            let glyph_row = (y - OVERLAY_Y) & 7;
            let glyph_col = x & 7;
            let ch = if line == 0 {
                overlay_char(text_col, available)
            } else {
                overlay_profile_char(text_col, generate, submit)
            };
            let bitmap = FONT8X8[(ch as usize - 1) * 8 + glyph_row];
            out[pos - offset] = if bitmap & (0x80 >> glyph_col) != 0 {
                235
            } else {
                16
            };
        }
    }

    let chroma_first_row = OVERLAY_Y / 2;
    let chroma_end_row = (OVERLAY_Y + 16) / 2;
    for row in chroma_first_row..chroma_end_row {
        let row_start = Y_PLANE_BYTES + row * WIDTH + OVERLAY_X;
        let row_end = row_start + OVERLAY_CHARS * 8;
        let start = offset.max(row_start);
        let stop = end.min(row_end);
        if start < stop {
            out[start - offset..stop - offset].fill(128);
        }
    }
}

fn overlay_char(index: usize, available: u8) -> u8 {
    if index < OVERLAY_TEXT.len() {
        return OVERLAY_TEXT[index];
    }
    let percent = available.min(100);
    match index - OVERLAY_TEXT.len() {
        0 => b'0' + percent / 100,
        1 => b'0' + (percent / 10) % 10,
        2 => b'0' + percent % 10,
        3 => b'%',
        _ => b' ',
    }
}

fn overlay_profile_char(index: usize, generate: u8, submit: u8) -> u8 {
    if index < OVERLAY_LINE_2_PREFIX.len() {
        return OVERLAY_LINE_2_PREFIX[index];
    }
    let index = index - OVERLAY_LINE_2_PREFIX.len();
    if index < 4 {
        return percent_char(index, generate);
    }
    let index = index - 4;
    if index < OVERLAY_LINE_2_MIDDLE.len() {
        return OVERLAY_LINE_2_MIDDLE[index];
    }
    percent_char(index - OVERLAY_LINE_2_MIDDLE.len(), submit)
}

fn percent_char(index: usize, percent: u8) -> u8 {
    let percent = percent.min(100);
    match index {
        0 => b'0' + percent / 100,
        1 => b'0' + (percent / 10) % 10,
        2 => b'0' + percent % 10,
        3 => b'%',
        _ => b' ',
    }
}
