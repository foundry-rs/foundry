use arrayvec::ArrayVec;
use bytes::{Bytes, BytesMut};
use ethnum::U256;
use fastrlp::*;
use hex_literal::hex;
use std::num::NonZeroUsize;

#[derive(Debug, PartialEq, Encodable, Decodable)]
struct Item {
    a: Bytes,
}

#[derive(Debug, PartialEq, Encodable, Decodable, MaxEncodedLen)]
struct Test4Numbers {
    a: u8,
    b: u64,
    c: U256,
    d: U256,
}

#[derive(Debug, PartialEq, EncodableWrapper, DecodableWrapper)]
pub struct W(Test4Numbers);

#[derive(Debug, PartialEq, Encodable)]
struct Test4NumbersGenerics<'a, D: Encodable> {
    a: u8,
    b: u64,
    c: &'a U256,
    d: &'a D,
}

fn encoded<T: Encodable>(t: &T) -> BytesMut {
    let mut out = BytesMut::new();
    t.encode(&mut out);
    out
}

#[test]
fn test_encode_item() {
    let item = Item {
        a: b"dog".to_vec().into(),
    };

    let expected = vec![0xc4, 0x83, b'd', b'o', b'g'];
    let out = encoded(&item);
    assert_eq!(&*out, expected);

    let decoded = Decodable::decode(&mut &*expected).expect("decode failure");
    assert_eq!(item, decoded);

    let item = Test4Numbers {
        a: 0x05,
        b: 0xdeadbeefbaadcafe,
        c: U256::from_be_bytes(hex!(
            "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
        )),
        d: U256::from_be_bytes(hex!(
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        )),
    };

    let expected = hex!("f84c0588deadbeefbaadcafea056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470").to_vec();
    let out = encoded(&item);
    assert_eq!(&*out, expected);

    let out = fastrlp::encode_fixed_size(&item);
    assert_eq!(&*out, expected);

    let decoded = Decodable::decode(&mut &*expected).unwrap();
    assert_eq!(item, decoded);

    let mut rlp_view = Rlp::new(&expected).unwrap();
    assert_eq!(rlp_view.get_next().unwrap(), Some(item.a));
    assert_eq!(rlp_view.get_next().unwrap(), Some(item.b));
    assert_eq!(rlp_view.get_next().unwrap(), Some(item.c));
    assert_eq!(rlp_view.get_next().unwrap(), Some(item.d));
    assert_eq!(rlp_view.get_next::<Bytes>().unwrap(), None);

    assert_eq!(
        encoded(&Test4NumbersGenerics {
            a: item.a,
            b: item.b,
            c: &item.c,
            d: &item.d
        }),
        expected
    );

    assert_eq!(encoded(&W(item)), expected);
    assert_eq!(W::decode(&mut &*expected).unwrap().0, decoded);
}

#[test]
fn invalid_decode_sideeffect() {
    let fixture = hex!("f84d0588deadbeefbaadcafea056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
    let mut sl: &[u8] = &fixture;

    assert_eq!(
        Test4Numbers::decode(&mut sl),
        Err(DecodeError::InputTooShort {
            needed: Some(NonZeroUsize::new(1).unwrap())
        })
    );

    assert_eq!(sl.len(), fixture.len());
}

#[test]
fn struct_equivalence() {
    let a = 0x05;
    let b = 0xadbeefbaadcafe;
    let c = U256::from_be_bytes(hex!(
        "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
    ));
    let d = U256::from_be_bytes(hex!(
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    ));

    let item = Test4Numbers { a, b, c, d };

    let vec: Vec<Bytes> = vec![
        zeroless_view(&a.to_be_bytes()).to_vec().into(),
        zeroless_view(&b.to_be_bytes()).to_vec().into(),
        zeroless_view(&c.to_be_bytes()).to_vec().into(),
        zeroless_view(&d.to_be_bytes()).to_vec().into(),
    ];

    let enc: &[u8] = &hex!("f84b0587adbeefbaadcafea056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");

    assert_eq!(encoded(&item), enc);
    assert_eq!(encoded(&vec), enc);
    assert_eq!(Test4Numbers::decode(&mut &*enc).unwrap(), item);
    assert_eq!(Vec::<Bytes>::decode(&mut &*enc).unwrap(), vec);
    assert!(ArrayVec::<Bytes, 3>::decode(&mut &*enc).is_err());
    assert_eq!(
        ArrayVec::<Bytes, 4>::decode(&mut &*enc).unwrap().as_slice(),
        vec
    );
}
