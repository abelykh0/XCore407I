# XCore407I Rust `no_std` blinker

Experimental Rust-based USB 2.0 High-Speed UVC camera running on an STM32F407. Implements real-time video generation and isochronous USB streaming 
using three 768-byte transactions per 125 µs microframe (2,304 bytes per microframe). USB polling is driven by a hardware timer interrupt at 8 kHz. 
Image data is rendered directly into a custom canvas and streamed without a full-frame copy. Supports 256×256 color and 512×256 monochrome rendering modes. 
Current prototype achieves 25 FPS.

## Build

Install the Rust target and build tools:

```powershell
rustup target add thumbv7em-none-eabihf
cargo install cargo-binutils
rustup component add llvm-tools
```

From this directory:

```powershell
cargo build --release --target thumbv7em-none-eabihf
probe-rs download --chip STM32F407IGTx target/thumbv7em-none-eabihf/release/xcore407i_blinker
probe-rs reset --chip STM32F407IGTx --connect-under-reset
```

Flash `target/thumbv7em-none-eabihf/release/xcore407i-blinker` (ELF) with an
SWD programmer such as ST-Link. The board does not include an onboard debugger.
This standalone image replaces the existing firmware while running.

## Hardware details

- MCU: STM32F407IGT6, Cortex-M4F, 1 MiB flash, 128 KiB SRAM.
- PHY: DP83848 at address 1, as configured in the existing firmware.
- MDC/MDIO: PC1/PA2, alternate function 11.
- The firmware configures the 8 MHz HSE to 168 MHz HCLK, matching the original
  project's PLL setup. The MII management clock uses the /102 divisor.
- PHY register `0x18` is placed in manual LED mode and its two LED bits are
  toggled every 500 ms while USB requests are polled continuously.

The implementation is `no_std`, uses no HAL or heap allocator, and uses
`cortex-m-rt`, `usb-device`, and `synopsys-usb-otg` for startup and USB device
support.

## Experimental OTG-HS internal DMA

The default build keeps the FIFO based USB driver path. To build the optional
STM32 OTG-HS internal DMA backend, use:

```powershell
cargo build --release --features usb-internal-dma
```

This feature uses the vendored `synopsys-usb-otg` driver fork and enables the
OTG controller's internal AHB DMA engine. It is separate from DMA in the
USB3300 PHY. DMA IN buffers are aligned `u32` arrays owned by the USB driver;
OUT and EP0 buffers come from statically allocated, `u32` aligned endpoint
memory in normal SRAM. The default FIFO implementation remains available by
building without the feature. Treat DMA mode as experimental until enumeration
and video capture have both been checked on the board.
