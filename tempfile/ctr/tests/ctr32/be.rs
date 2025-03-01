//! Counter Mode with a 32-bit big endian counter

use cipher::{KeyIvInit, StreamCipher, StreamCipherSeek, StreamCipherSeekCore};
use hex_literal::hex;

type Aes128Ctr = ctr::Ctr32BE<aes::Aes128>;

const KEY: &[u8; 16] = &hex!("000102030405060708090A0B0C0D0E0F");
const NONCE1: &[u8; 16] = &hex!("11111111111111111111111111111111");
const NONCE2: &[u8; 16] = &hex!("222222222222222222222222FFFFFFFE");

#[test]
fn counter_incr() {
    let mut ctr = Aes128Ctr::new(KEY.into(), NONCE1.into());
    assert_eq!(ctr.get_core().get_block_pos(), 0);

    let mut buffer = [0u8; 64];
    ctr.apply_keystream(&mut buffer);

    assert_eq!(ctr.get_core().get_block_pos(), 4);
    assert_eq!(
        &buffer[..],
        &hex!(
            "35D14E6D3E3A279CF01E343E34E7DED36EEADB04F42E2251AB4377F257856DBA"
            "0AB37657B9C2AA09762E518FC9395D5304E96C34CCD2F0A95CDE7321852D90C0"
        )[..]
    );
}

#[test]
fn counter_seek() {
    let mut ctr = Aes128Ctr::new(KEY.into(), NONCE1.into());
    ctr.seek(16);
    assert_eq!(ctr.get_core().get_block_pos(), 1);

    let mut buffer = [0u8; 64];
    ctr.apply_keystream(&mut buffer);

    assert_eq!(ctr.get_core().get_block_pos(), 5);
    assert_eq!(
        &buffer[..],
        &hex!(
            "6EEADB04F42E2251AB4377F257856DBA0AB37657B9C2AA09762E518FC9395D53"
            "04E96C34CCD2F0A95CDE7321852D90C0F7441EAB3811A03FDBD162AEC402F5AA"
        )[..]
    );
}

#[test]
fn keystream_xor() {
    let mut ctr = Aes128Ctr::new(KEY.into(), NONCE1.into());
    let mut buffer = [1u8; 64];

    ctr.apply_keystream(&mut buffer);
    assert_eq!(
        &buffer[..],
        &hex!(
            "34D04F6C3F3B269DF11F353F35E6DFD26FEBDA05F52F2350AA4276F356846CBB"
            "0BB27756B8C3AB08772F508EC8385C5205E86D35CDD3F1A85DDF7220842C91C1"
        )[..]
    );
}

#[test]
fn counter_wrap() {
    let mut ctr = Aes128Ctr::new(KEY.into(), NONCE2.into());
    assert_eq!(ctr.get_core().get_block_pos(), 0);

    let mut buffer = [0u8; 64];
    ctr.apply_keystream(&mut buffer);

    assert_eq!(ctr.get_core().get_block_pos(), 4);
    assert_eq!(
        &buffer[..],
        &hex!(
            "58FC849D1CF53C54C63E1B1D15CB3C8AAA335F72135585E9FF943F4DAC77CB63"
            "BD1AE8716BE69C3B4D886B222B9B4E1E67548EF896A96E2746D8CA6476D8B183"
        )[..]
    );
}

cipher::iv_state_test!(
    iv_state,
    ctr::CtrCore<aes::Aes128, ctr::flavors::Ctr32BE>,
    apply_ks,
);
