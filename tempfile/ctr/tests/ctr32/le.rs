//! Counter Mode with a 32-bit little endian counter

use cipher::{
    consts::U16, generic_array::GenericArray, KeyIvInit, StreamCipher, StreamCipherSeek,
    StreamCipherSeekCore,
};
use hex_literal::hex;

type Aes128Ctr = ctr::Ctr32LE<aes::Aes128>;

const KEY: &[u8; 16] = &hex!("000102030405060708090A0B0C0D0E0F");
const NONCE1: &[u8; 16] = &hex!("11111111111111111111111111111111");
const NONCE2: &[u8; 16] = &hex!("FEFFFFFF222222222222222222222222");

/// Compute nonce as used by AES-GCM-SIV
fn nonce(bytes: &[u8; 16]) -> GenericArray<u8, U16> {
    let mut n = *bytes;
    n[15] |= 0x80;
    n.into()
}

#[test]
fn counter_incr() {
    let mut ctr = Aes128Ctr::new(KEY.into(), &nonce(NONCE1));
    assert_eq!(ctr.get_core().get_block_pos(), 0);

    let mut buffer = [0u8; 64];
    ctr.apply_keystream(&mut buffer);

    // assert_eq!(ctr.current_ctr(), 4);
    assert_eq!(
        &buffer[..],
        &hex!(
            "2A0680B210CAD45E886D7EF6DAB357C9F18B39AFF6930FDB2D9FCE34261FF699"
            "EB96774669D24B560C9AD028C5C39C4580775A82065256B4787DC91C6942B700"
        )[..]
    );
}

#[test]
fn counter_seek() {
    let mut ctr = Aes128Ctr::new(KEY.into(), &nonce(NONCE1));
    ctr.seek(16);
    assert_eq!(ctr.get_core().get_block_pos(), 1);

    let mut buffer = [0u8; 64];
    ctr.apply_keystream(&mut buffer);

    assert_eq!(ctr.get_core().get_block_pos(), 5);
    assert_eq!(
        &buffer[..],
        &hex!(
            "F18B39AFF6930FDB2D9FCE34261FF699EB96774669D24B560C9AD028C5C39C45"
            "80775A82065256B4787DC91C6942B7001564DDA1B07DCED9201AB71BAF06905B"
        )[..]
    );
}

#[test]
fn keystream_xor() {
    let mut ctr = Aes128Ctr::new(KEY.into(), &nonce(NONCE1));
    let mut buffer = [1u8; 64];

    ctr.apply_keystream(&mut buffer);
    assert_eq!(
        &buffer[..],
        &hex!(
            "2B0781B311CBD55F896C7FF7DBB256C8F08A38AEF7920EDA2C9ECF35271EF798"
            "EA97764768D34A570D9BD129C4C29D4481765B83075357B5797CC81D6843B601"
        )[..]
    );
}

#[test]
fn counter_wrap() {
    let mut ctr = Aes128Ctr::new(KEY.into(), &nonce(NONCE2));
    assert_eq!(ctr.get_core().get_block_pos(), 0);

    let mut buffer = [0u8; 64];
    ctr.apply_keystream(&mut buffer);

    assert_eq!(ctr.get_core().get_block_pos(), 4);
    assert_eq!(
        &buffer[..],
        &hex!(
            "A1E649D8B382293DC28375C42443BB6A226BAADC9E9CCA8214F56E07A4024E06"
            "6355A0DA2E08FB00112FFA38C26189EE55DD5B0B130ED87096FE01B59A665A60"
        )[..]
    );
}

cipher::iv_state_test!(
    iv_state,
    ctr::CtrCore<aes::Aes128, ctr::flavors::Ctr32LE>,
    apply_ks,
);
