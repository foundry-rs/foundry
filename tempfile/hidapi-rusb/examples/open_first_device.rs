/****************************************************************************
    Copyright (c) 2018 Roland Ruckerbauer All Rights Reserved.

    This file is part of hidapi-rs, based on hidapi-rs by Osspial
****************************************************************************/

//! Opens the first hid device it can find, and reads data in a blocking fashion
//! from it in an endless loop.

#[cfg(all(feature = "linux-static-rusb", not(target_os = "macos")))]
extern crate rusb;

extern crate hidapi_rusb;

use hidapi_rusb::{HidApi, HidError};

fn main() {
    fn run() -> Result<(), HidError> {
        let hidapi = HidApi::new()?;

        let device_info = hidapi
            .device_list()
            .next()
            .expect("No devices are available!")
            .clone();

        println!(
            "Opening device:\n VID: {:04x}, PID: {:04x}\n",
            device_info.vendor_id(),
            device_info.product_id()
        );

        let device = device_info.open_device(&hidapi)?;

        let mut buf = vec![0; 64];

        println!("Reading data from device ...\n");

        loop {
            let len = device.read(&mut buf)?;
            println!("{:?}", &buf[..len]);
        }
    }

    if let Err(e) = run() {
        eprintln!("Error: {}", e);
    }
}
