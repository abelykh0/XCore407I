use crate::camera;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 256;

struct RotozoomState {
    frame: usize,
    y: usize,
}

static mut STATE: RotozoomState = RotozoomState { frame: 0, y: 0 };

/// Renders the next scanline row, automatically tracking the current row and frame,
/// and plotting each pixel using `set_demo_pixel`.
pub fn render_next() {
    let (frame, y) = unsafe {
        let f = STATE.frame;
        let y = STATE.y;

        // Advance to the next row; wrap around and increment frame at the bottom of the screen
        STATE.y += 1;
        if STATE.y >= HEIGHT {
            STATE.y = 0;
            STATE.frame += 1;
        }
        (f, y)
    };

    // Animation parameters derived from the current frame
    let (sin_val, cos_val) = unsafe {
        (
            libm::sinf(frame as f32 / 50.0),
            libm::cosf(frame as f32 / 100.0),
        )
    };

    let zoom = sin_val * 0.7 + 1.0;
    let translation_x = cos_val * 100.0;
    let angle = frame as f32 * 0.005;

    #[cfg(target_os = "none")]
    let (cos_a, sin_a) = (libm::cosf(angle), libm::sinf(angle));
    #[cfg(not(target_os = "none"))]
    let (cos_a, sin_a) = (angle.cos(), angle.sin());

    let c_rot = cos_a * zoom;
    let s_rot = sin_a * zoom;

    const COLS: f32 = WIDTH as f32;
    const ROWS: f32 = HEIGHT as f32;
    let cx = COLS / 2.0;
    let cy = ROWS / 2.0;

    let tx = translation_x;
    let ty = 0.0;

    let fy = y as f32 - cy;

    // Compute and plot pixels for this single row directly
    for x in 0..WIDTH {
        let fx = x as f32 - cx;

        let u = c_rot * fx - s_rot * fy + tx + cx;
        let v = s_rot * fx + c_rot * fy + ty + cy;

        let color = tex_fetch(u, v);
        
        camera::set_demo_pixel(x, y, color);
    }
}

/// Fetches a texel from a 256x256 canvas space using RGB222 (64 colors).
#[inline(always)]
fn tex_fetch(u: f32, v: f32) -> u8 {
    let x = (u as i32) & 0xFF;
    let y = (v as i32) & 0xFF;

    let r = ((x >> 6) ^ (y >> 6)) & 0x3;
    let g = (x >> 4) & 0x3;
    let b = (y >> 4) & 0x3;

    ((r << 4) | (g << 2) | b) as u8
}