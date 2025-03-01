/****************************************************************************
    Copyright (c) 2015 Osspial All Rights Reserved.

    This file is part of hidapi-rs, based on hidapi_rust by Roland Ruckerbauer.
****************************************************************************/

//! Opens a Thrustmaster T-Flight HOTAS X HID and reads data from it. This
//! example will not work unless such an HID is plugged in to your system.
//! Will update in the future to support all HIDs.

#[cfg(all(feature = "linux-static-rusb", not(target_os = "macos")))]
extern crate rusb;

extern crate hidapi_rusb;
use hidapi_rusb::HidApi;

fn main() {
    let api = HidApi::new().expect("Failed to create API instance");

    let joystick = api.open(1103, 45320).expect("Failed to open device");

    loop {
        let mut buf = [0u8; 256];
        let res = joystick.read(&mut buf[..]).unwrap();

        let mut data_string = String::new();

        for u in &buf[..res] {
            data_string.push_str(&(u.to_string() + "\t"));
        }

        println!("{}", data_string);
    }
}
