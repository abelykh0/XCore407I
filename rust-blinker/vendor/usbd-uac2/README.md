![Crates.io Version](https://img.shields.io/crates/v/usbd-uac2) ![docs.rs](https://img.shields.io/docsrs/usbd-uac2)

# usbd-uac2

USB Audio Class 2.0

This crate provides a USB Audio Class 2.0 implementation for
[usb-device](https://crates.io/crates/usb-device). It implements all
required elements of the specification, however many controls are not
implemented (e.g. mixers, effects).

Device behaviour is driven by implementing the `ClockSource` and
`AudioHandler` traits to configure the audio pipeline and source/sink data
to/from USB.

Example (creates a UAC2 device with in and out streams):

```rust
let mut audio = YourTraitImpl {...};

let config = UsbAudioClassConfig::new(usb_speed, FunctionCode::IoBox, &mut audio)
    // base_id is USB entity ID, id 1 is always taken by the clock source, and each stream builds 2 entities
    .with_output_config(TerminalConfig::builder().base_id(2).build())
    .with_input_config(TerminalConfig::builder().base_id(4).build());

let mut uac2 = config.build(&usb_bus).unwrap();

let mut usb_dev = usbd_uac2::builder(&usb_bus, UsbVidPid(0x1209, 0x0001))
    .strings(&[StringDescriptors::default()
        .manufacturer("usbd_uac2")
        .product("example")])
    .unwrap()
    .max_packet_size_0(64) // Required to be 64 on HS
    .unwrap()
    .build();
```

No work needs to be done in the poll loop, the class implementation will
call your trait callbacks as required, just call the usb poll as usual:

```rust
loop {
    usb_dev.poll(&mut [&mut uac2]);
}
```

See the trait documentation or examples for additional details.
