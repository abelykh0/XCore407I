# XCore407I Rust `no_std` blinker

This is the standalone Rust firmware project for the Waveshare XCore407I with
an STM32F407IGT6. It retains the Ethernet LED blinker and now includes a
generated 512x512 NV12 color-bar image and a `no_std` UVC 1.1 class layer,
matching the original project's static video mode.

The board's Ethernet connector indicators are controlled over the STM32
Ethernet MAC's MDIO/MDC interface; they are not directly connected to MCU GPIO
pins. The `camera` module generates the frame on demand, so it does not reserve
the 384 KiB needed for a full NV12 image in SRAM.

The firmware now runs the Ethernet PHY blinker while polling the OTG-HS device
controller. `usb_uvc` supplies a custom F407 `UsbPeripheral` adapter for the
USB3300 ULPI pins and a `usb-device` UVC class. The camera advertises a single
512x512 NV12 frame at 12 fps, handles 26- and 34-byte Probe/Commit requests, and
sends the compact demo image over high-speed isochronous IN endpoint `0x81`
(two 1024-byte transactions per microframe). Build with
`--features uvc-single-transaction` for the one-transaction fallback. The host
should identify it as a USB video
class camera; the development USB ID is `1209:4071`.

## Build

Install the Rust target and build tools:

```powershell
rustup target add thumbv7em-none-eabihf
cargo install cargo-binutils
rustup component add llvm-tools
```

From this directory:

```powershell
cargo build --release
cargo objcopy --release -- -O binary firmware.bin
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
