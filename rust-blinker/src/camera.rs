//! C-style compact color canvas and NV12 UVC packetizer.
//!
//! A 256x256 palette-index canvas lives in SRAM. Each source color expands to
//! a 2x2 NV12 output block, so there is no 384 KiB frame buffer in RAM.

use crate::palette::{UV_PALETTE, Y_PALETTE};

pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 512;
pub const Y_PLANE_BYTES: usize = WIDTH * HEIGHT;
pub const FRAME_BYTES: usize = Y_PLANE_BYTES + (WIDTH * HEIGHT / 2);
pub const UVC_HEADER_BYTES: usize = 4;
// Match the original C packetizer: one 512-pixel NV12 row's worth of payload
// (512 Y bytes + 256 UV bytes) plus the four-byte UVC header.
pub const PAYLOAD_BYTES: usize = WIDTH * 3 / 2;
pub const TRANSFER_BYTES: usize = UVC_HEADER_BYTES + PAYLOAD_BYTES;

pub const CANVAS_WIDTH: usize = WIDTH / 2;
pub const CANVAS_HEIGHT: usize = HEIGHT / 2;
pub const CANVAS_BYTES: usize = CANVAS_WIDTH * CANVAS_HEIGHT;

// One 8-bit palette index per 2x2 output block. This is 64 KiB rather than a
// 384 KiB NV12 frame; the USB packet is assembled from two 512-byte row caches.
static mut DEMO_CANVAS: [u8; CANVAS_BYTES] = [0; CANVAS_BYTES];

/// Initialize a colorful image in the compact canvas. Call once before USB
/// streaming; later calls to `set_demo_pixel` can update individual source pixels.
pub fn initialize_demo_canvas() {
    let canvas = core::ptr::addr_of_mut!(DEMO_CANVAS).cast::<u8>();
    let mut y = 0;
    while y < CANVAS_HEIGHT {
        let mut x = 0;
        while x < CANVAS_WIDTH {
            // 8x8 color chart, matching the original C palette demo.
            let mut color = ((y / 32) * 8 + x / 32) as u8;

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
            unsafe { canvas.add(y * CANVAS_WIDTH + x).write(color) };
            x += 1;
        }
        y += 1;
    }
}

/// Change one logical color pixel. Every source pixel expands to a 2x2 block
/// in the 512x512 NV12 stream, as in the original C color mode.
pub fn set_demo_pixel(x: usize, y: usize, color: u8) {
    if x < CANVAS_WIDTH && y < CANVAS_HEIGHT {
        let canvas = core::ptr::addr_of_mut!(DEMO_CANVAS).cast::<u8>();
        unsafe { canvas.add(y * CANVAS_WIDTH + x).write(color & 0x3f) };
    }
}

#[inline(always)]
fn demo_pixel(index: usize) -> u8 {
    let canvas = core::ptr::addr_of!(DEMO_CANVAS).cast::<u8>();
    unsafe { canvas.add(index).read() }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Packet {
    pub len: usize,
    pub frame_complete: bool,
}

/// Emits the same compact demo image repeatedly as UVC payloads.
pub struct Nv12DemoImage {
    offset: usize,
    frame_id: bool,
    cpu_available_percent: u8,
    cpu_generate_percent: u8,
    cpu_submit_percent: u8,
    cached_y_source_row: u16,
    cached_uv_source_row: u16,
    y_row: [u8; WIDTH],
    uv_row: [u8; WIDTH],
}

impl Nv12DemoImage {
    pub const fn new() -> Self {
        Self {
            offset: 0,
            frame_id: false,
            cpu_available_percent: 100,
            cpu_generate_percent: 0,
            cpu_submit_percent: 0,
            cached_y_source_row: u16::MAX,
            cached_uv_source_row: u16::MAX,
            y_row: [0; WIDTH],
            uv_row: [0; WIDTH],
        }
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
        let data_len = remaining
            .min(transfer_capacity - UVC_HEADER_BYTES);
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
        overlay_cpu_text(
            self.offset,
            &mut out[UVC_HEADER_BYTES..UVC_HEADER_BYTES + data_len],
            self.cpu_available_percent,
            self.cpu_generate_percent,
            self.cpu_submit_percent,
        );

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

impl Default for Nv12DemoImage {
    fn default() -> Self {
        Self::new()
    }
}

impl Nv12DemoImage {
    /// Render one sequential NV12 byte range using one cached row per plane.
    /// The luma row is doubled horizontally and vertically from the compact
    /// 256x256 color canvas, like the original C color renderer.
    fn fill_nv12(&mut self, offset: usize, out: &mut [u8]) {
        let mut pos = offset;
        let mut written = 0;
        while written < out.len() && pos < FRAME_BYTES {
            if pos < Y_PLANE_BYTES {
                let output_row = pos / WIDTH;
                let x = pos % WIDTH;
                let source_row = (output_row / 2) as u16;
                self.prepare_y_row(source_row);
                let run = (WIDTH - x)
                    .min(Y_PLANE_BYTES - pos)
                    .min(out.len() - written);
                out[written..written + run].copy_from_slice(&self.y_row[x..x + run]);
                pos += run;
                written += run;
            } else {
                let chroma_pos = pos - Y_PLANE_BYTES;
                let source_row = (chroma_pos / WIDTH) as u16;
                let x = chroma_pos % WIDTH;
                self.prepare_uv_row(source_row);
                let run = (WIDTH - x).min(FRAME_BYTES - pos).min(out.len() - written);
                out[written..written + run].copy_from_slice(&self.uv_row[x..x + run]);
                pos += run;
                written += run;
            }
        }
        out[written..].fill(0);
    }

    fn prepare_y_row(&mut self, source_row: u16) {
        if self.cached_y_source_row == source_row {
            return;
        }
        let base = source_row as usize * CANVAS_WIDTH;
        let mut x = 0;
        while x < CANVAS_WIDTH {
            let y = Y_PALETTE[demo_pixel(base + x) as usize];
            self.y_row[x * 2] = y;
            self.y_row[x * 2 + 1] = y;
            x += 1;
        }
        self.cached_y_source_row = source_row;
    }

    fn prepare_uv_row(&mut self, source_row: u16) {
        if self.cached_uv_source_row == source_row {
            return;
        }
        let base = source_row as usize * CANVAS_WIDTH;
        let mut x = 0;
        while x < CANVAS_WIDTH {
            let uv = UV_PALETTE[demo_pixel(base + x) as usize];
            self.uv_row[x * 2] = uv[0];
            self.uv_row[x * 2 + 1] = uv[1];
            x += 1;
        }
        self.cached_uv_source_row = source_row;
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
