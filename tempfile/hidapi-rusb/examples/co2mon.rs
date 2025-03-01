/****************************************************************************
    Copyright (c) 2015 Artyom Pavlov All Rights Reserved.

    This file is part of hidapi-rs, based on hidapi_rust by Roland Ruckerbauer.
    It's also based on the Oleg Bulatov's work (https://github.com/dmage/co2mon)
****************************************************************************/

//! Opens a KIT MT 8057 CO2 detector and reads data from it. This
//! example will not work unless such an HID is plugged in to your system.

#[cfg(all(feature = "linux-static-rusb", not(target_os = "macos")))]
extern crate rusb;

extern crate hidapi_rusb;
use hidapi_rusb::{HidApi, HidDevice};
use std::thread::sleep;
use std::time::Duration;

const CODE_TEMPERATURE: u8 = 0x42;
const CODE_CONCENTRATION: u8 = 0x50;
const HID_TIMEOUT: i32 = 5000;
const RETRY_SEC: u64 = 1;
const DEV_VID: u16 = 0x04d9;
const DEV_PID: u16 = 0xa052;
const PACKET_SIZE: usize = 8;

enum CO2Result {
    Temperature(f32),
    Concentration(u16),
    Unknown(u8, u16),
    Error(&'static str),
}

fn decode_temperature(value: u16) -> f32 {
    (value as f32) * 0.0625 - 273.15
}

fn decode_buf(buf: [u8; PACKET_SIZE]) -> CO2Result {
    let mut res: [u8; PACKET_SIZE] = [
        (buf[3] << 5) | (buf[2] >> 3),
        (buf[2] << 5) | (buf[4] >> 3),
        (buf[4] << 5) | (buf[0] >> 3),
        (buf[0] << 5) | (buf[7] >> 3),
        (buf[7] << 5) | (buf[1] >> 3),
        (buf[1] << 5) | (buf[6] >> 3),
        (buf[6] << 5) | (buf[5] >> 3),
        (buf[5] << 5) | (buf[3] >> 3),
    ];

    let magic_word = b"Htemp99e";
    for i in 0..PACKET_SIZE {
        let sub_val: u8 = (magic_word[i] << 4) | (magic_word[i] >> 4);
        res[i] = u8::overflowing_sub(res[i], sub_val).0;
    }

    if res[4] != 0x0d {
        return CO2Result::Error("Unexpected data (data[4] != 0x0d)");
    }
    let checksum = u8::overflowing_add(u8::overflowing_add(res[0], res[1]).0, res[2]).0;
    if checksum != res[3] {
        return CO2Result::Error("Checksum error");
    }

    let val: u16 = ((res[1] as u16) << 8) + res[2] as u16;
    match res[0] {
        CODE_TEMPERATURE => CO2Result::Temperature(decode_temperature(val)),
        CODE_CONCENTRATION => {
            if val > 3000 {
                CO2Result::Error("Concentration bigger than 3000 (uninitialized device?)")
            } else {
                CO2Result::Concentration(val)
            }
        }
        _ => CO2Result::Unknown(res[0], val),
    }
}

fn open_device(api: &HidApi) -> HidDevice {
    loop {
        match api.open(DEV_VID, DEV_PID) {
            Ok(dev) => return dev,
            Err(err) => {
                println!("{}", err);
                sleep(Duration::from_secs(RETRY_SEC));
            }
        }
    }
}

fn main() {
    let api = HidApi::new().expect("HID API object creation failed");

    let dev = open_device(&api);

    dev.send_feature_report(&[0; PACKET_SIZE])
        .expect("Feature report failed");

    println!(
        "Manufacurer:\t{:?}",
        dev.get_manufacturer_string()
            .expect("Failed to read manufacurer string")
    );
    println!(
        "Product:\t{:?}",
        dev.get_product_string()
            .expect("Failed to read product string")
    );
    println!(
        "Serial number:\t{:?}",
        dev.get_serial_number_string()
            .expect("Failed to read serial number")
    );

    loop {
        let mut buf = [0; PACKET_SIZE];
        match dev.read_timeout(&mut buf[..], HID_TIMEOUT) {
            Ok(PACKET_SIZE) => (),
            Ok(res) => {
                println!("Error: unexpected length of data: {}/{}", res, PACKET_SIZE);
                continue;
            }
            Err(err) => {
                println!("Error: {:}", err);
                sleep(Duration::from_secs(RETRY_SEC));
                continue;
            }
        }
        match decode_buf(buf) {
            CO2Result::Temperature(val) => println!("Temp:\t{:?}", val),
            CO2Result::Concentration(val) => println!("Conc:\t{:?}", val),
            CO2Result::Unknown(..) => (),
            CO2Result::Error(val) => {
                println!("Error:\t{}", val);
                sleep(Duration::from_secs(RETRY_SEC));
            }
        }
    }
}
