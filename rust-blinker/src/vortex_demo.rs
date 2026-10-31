use core::sync::atomic::{AtomicUsize, Ordering};
use crate::camera;

const WIDTH: usize = 256;
const HEIGHT: usize = 256;

// Internal frame counter managed automatically across calls
static FRAME_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Renders the next frame of the Vortex Wrap effect using camera::set_demo_pixel.
pub fn render_next_frame() {
    let frame_idx = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
    
    let width_f = WIDTH as f32;
    let height_f = HEIGHT as f32;
    let cx = width_f / 2.0;
    let cy = height_f / 2.0;

    // Time step scaled for 25 fps target
    let time = frame_idx as f32 * (1.0 / 25.0);

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;

            // Polar coordinates using libm math functions for no_std
            let r_dist = libm::sqrtf(dx * dx + dy * dy);
            let theta = libm::atan2f(dy, dx);

            // Vortex / Twirl distortion formula (classic demoscene warp)
            let swirl = 18.0 / (r_dist + 2.0) + time * 1.5;
            let theta_prime = theta + swirl;

            // Back-map to texture coordinates with time offset for motion
            let u = r_dist * libm::cosf(theta_prime) + time * 25.0;
            let v = r_dist * libm::sinf(theta_prime) + time * 25.0;

            // Procedural color generation quantized to RGB222 (2 bits per channel -> values 0..3)
            let r_val = ((libm::sinf(u * 0.08) * 0.5 + 0.5) * 3.0).clamp(0.0, 3.0) as u8;
            let g_val = ((libm::cosf(v * 0.08) * 0.5 + 0.5) * 3.0).clamp(0.0, 3.0) as u8;
            let b_val = ((libm::sinf((u + v) * 0.05) * 0.5 + 0.5) * 3.0).clamp(0.0, 3.0) as u8;

            // Pack into 64-color RGB222 format (RR GG BB -> bits 5:4, 3:2, 1:0)
            let rgb222 = (r_val << 4) | (g_val << 2) | b_val;

            // Write pixel directly via hardware wrapper
            camera::set_demo_pixel(x, y, rgb222);
        }
    }
}