# XCore407I Rust `no_std` blinker

Experimental Rust-based USB 2.0 High-Speed UVC camera running on an STM32F407. Implements real-time video generation and isochronous USB streaming 
using three 768-byte transactions per 125 µs microframe (2,304 bytes per microframe). USB polling is driven by a hardware timer interrupt at 8 kHz. 
Image data is rendered directly into a custom canvas and streamed without a full-frame copy. Supports 256×256 color and 512×256 monochrome rendering modes. 
Current prototype achieves 25 FPS.

## Build

From this directory:

```powershell
cargo build --release --target thumbv7em-none-eabihf
probe-rs download --chip STM32F407IGTx target/thumbv7em-none-eabihf/release/xcore407i_blinker
//probe-rs reset --chip STM32F407IGTx --connect-under-reset
```
