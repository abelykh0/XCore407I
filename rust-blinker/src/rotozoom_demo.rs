
use crate::camera;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 256 - 8;

struct RotozoomState {
    frame: usize,
    y: usize,

    // Transformation coefficients, updated once per frame.
    c_rot: f32,
    s_rot: f32,
    tx: f32,
}

impl RotozoomState {
    const fn new() -> Self {
        Self {
            frame: 0,
            y: 0,
            c_rot: 1.0,
            s_rot: 0.0,
            tx: 0.0,
        }
    }

    #[inline]
    fn update_transform(&mut self) {
        let frame = self.frame as f32;

        let zoom = libm::sinf(frame / 50.0) * 0.7 + 1.0;
        self.tx = libm::cosf(frame / 100.0) * 100.0;

        let angle = frame * 0.005;

        self.c_rot = libm::cosf(angle) * zoom;
        self.s_rot = libm::sinf(angle) * zoom;
    }
}

static mut STATE: RotozoomState = RotozoomState::new();

pub fn render_next() {
    // This renderer is called from the main loop, not an interrupt.
    let state = unsafe {
        &mut *core::ptr::addr_of_mut!(STATE)
    };

    // Update animation parameters at the beginning of each frame.
    if state.y == 0 {
        state.update_transform();
    }

    let y = state.y;

    const CX: f32 = WIDTH as f32 / 2.0;
    const CY: f32 = HEIGHT as f32 / 2.0;

    let fy = y as f32 - CY;

    let c_rot = state.c_rot;
    let s_rot = state.s_rot;

    // Texture coordinate of the first pixel in this scanline.
    let mut u = -c_rot * CX
        - s_rot * fy
        + state.tx
        + CX;

    let mut v = -s_rot * CX
        + c_rot * fy
        + CY;

    // Render one scanline using incremental coordinate updates.
    for x in 0..WIDTH {
        let color = tex_fetch(u, v);

        camera::set_demo_pixel(x, y, color);

        u += c_rot;
        v += s_rot;
    }

    // Advance scanline/frame state.
    state.y += 1;

    if state.y >= HEIGHT {
        state.y = 0;
        state.frame += 1;
    }
}

#[inline(always)]
fn tex_fetch(u: f32, v: f32) -> u8 {
    let x = (u as i32) & 0xFF;
    let y = (v as i32) & 0xFF;

    let r = ((x >> 4) ^ (y >> 4)) & 0x3;
    let g = (x >> 2) & 0x3;
    let b = (y >> 2) & 0x3;

    ((r << 4) | (g << 2) | b) as u8
}
