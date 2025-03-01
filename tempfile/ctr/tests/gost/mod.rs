use cipher::{KeyIvInit, StreamCipher};
use hex_literal::hex;

type MagmaCtr = ctr::Ctr32BE<magma::Magma>;
type KuznyechikCtr = ctr::Ctr64BE<kuznyechik::Kuznyechik>;

/// Test vectors from GOST R 34.13-2015 (Section A.1.2)
#[test]
fn kuznyechik() {
    let key = hex!(
        "8899aabbccddeeff0011223344556677"
        "fedcba98765432100123456789abcdef"
    );
    let nonce = hex!("1234567890abcef00000000000000000");
    let mut pt = hex!(
        "1122334455667700ffeeddccbbaa9988"
        "00112233445566778899aabbcceeff0a"
        "112233445566778899aabbcceeff0a00"
        "2233445566778899aabbcceeff0a0011"
    );
    let ct = hex!(
        "f195d8bec10ed1dbd57b5fa240bda1b8"
        "85eee733f6a13e5df33ce4b33c45dee4"
        "a5eae88be6356ed3d5e877f13564a3a5"
        "cb91fab1f20cbab6d1c6d15820bdba73"
    );
    let mut cipher = KuznyechikCtr::new(&key.into(), &nonce.into());
    cipher.apply_keystream(&mut pt);
    assert_eq!(pt[..], ct[..]);
}

/// Test vectors from GOST R 34.13-2015 (Section A.2.2)
#[test]
fn magma() {
    let key = hex!(
        "ffeeddccbbaa99887766554433221100"
        "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"
    );
    let nonce = hex!("1234567800000000");
    let mut pt = hex!(
        "92def06b3c130a59"
        "db54c704f8189d20"
        "4a98fb2e67a8024c"
        "8912409b17b57e41"
    );
    let ct = hex!(
        "4e98110c97b7b93c"
        "3e250d93d6e85d69"
        "136d868807b2dbef"
        "568eb680ab52a12d"
    );
    let mut cipher = MagmaCtr::new(&key.into(), &nonce.into());
    cipher.apply_keystream(&mut pt);
    assert_eq!(pt[..], ct[..]);
}
