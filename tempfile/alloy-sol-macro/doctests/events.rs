#![allow(clippy::assertions_on_constants)]

use alloy_primitives::{hex, keccak256, Bytes, Log, B256, U256};
use alloy_rlp::{Decodable, Encodable};
use alloy_sol_types::{abi::token::WordToken, sol, SolEvent};

sol! {
    #[derive(Debug, Default, PartialEq)]
    event MyEvent(bytes32 indexed a, uint256 b, string indexed c, bytes d);

    event LogNote(
        bytes4   indexed  sig,
        address  indexed  guy,
        bytes32  indexed  foo,
        bytes32  indexed  bar,
        uint              wad,
        bytes             fax
    ) anonymous;

    struct Data {
        bytes data;
    }
    event MyEvent2(Data indexed data);
}

#[test]
fn event() {
    assert_event_signature::<MyEvent>("MyEvent(bytes32,uint256,string,bytes)");
    assert!(!MyEvent::ANONYMOUS);
    let event = MyEvent {
        a: [0x11; 32].into(),
        b: U256::from(1u64),
        c: keccak256("Hello World"),
        d: Bytes::default(),
    };
    // topics are `(SELECTOR, a, keccak256(c))`
    assert_eq!(
        event.encode_topics_array::<3>(),
        [
            WordToken(MyEvent::SIGNATURE_HASH),
            WordToken(B256::repeat_byte(0x11)),
            WordToken(keccak256("Hello World"))
        ]
    );
    // dynamic data is `abi.abi_encode(b, d)`
    assert_eq!(
        event.encode_data(),
        hex!(
            // b
            "0000000000000000000000000000000000000000000000000000000000000001"
            // d offset
            "0000000000000000000000000000000000000000000000000000000000000040"
            // d length
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
    );

    assert_event_signature::<LogNote>("LogNote(bytes4,address,bytes32,bytes32,uint256,bytes)");
    assert!(LogNote::ANONYMOUS);

    assert_event_signature::<MyEvent2>("MyEvent2((bytes))");
    assert!(!MyEvent2::ANONYMOUS);
}

#[test]
fn event_rlp_roundtrip() {
    let event = MyEvent {
        a: [0x11; 32].into(),
        b: U256::from(1u64),
        c: keccak256("Hello World"),
        d: Vec::new().into(),
    };

    let rlpable_log = Log::<MyEvent>::new_from_event_unchecked(Default::default(), event);

    let mut rlp_encoded = vec![];
    rlpable_log.encode(&mut rlp_encoded);
    assert_eq!(rlpable_log.length(), rlp_encoded.len());

    let rlp_decoded = Log::decode(&mut rlp_encoded.as_slice()).unwrap();
    assert_eq!(rlp_decoded, rlpable_log.reserialize());

    let decoded_log = MyEvent::decode_log(&rlp_decoded, true).unwrap();

    assert_eq!(decoded_log, rlpable_log)
}

fn assert_event_signature<T: SolEvent>(expected: &str) {
    assert_eq!(T::SIGNATURE, expected);
    assert_eq!(T::SIGNATURE_HASH, keccak256(expected));
}
