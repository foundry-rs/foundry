use serial_test::serial;

use coins_ledger::{
    common::*,
    transports::{self, native::hid, LedgerAsync},
};

#[test]
#[serial]
#[ignore]
fn ledger_device_path() {
    let transport = hid::TransportNativeHID::new().unwrap();

    // TODO: Extend to discover two devices
    let manufacturer = transport.get_manufacturer_string();
    println!("{manufacturer:?}");
}

#[tokio::test]
#[serial]
#[ignore]
async fn exchange() {
    let transport = transports::Ledger::init()
        .await
        .expect("Could not get a device");
    let buf: &[u8] = &[];
    // Ethereum `get_app_version`
    let command = APDUCommand {
        cla: 0xe0,
        ins: 0x06,
        p1: 0x00,
        p2: 0x00,
        data: buf.into(),
        response_len: None,
    };
    let result = transport.exchange(&command).await.unwrap();
    println!("{result}");
}
