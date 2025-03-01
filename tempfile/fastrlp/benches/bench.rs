use bytes::BytesMut;
use criterion::{criterion_group, criterion_main, Criterion};
use ethnum::*;
use fastrlp::*;
use hex_literal::hex;

fn bench_encode(c: &mut Criterion) {
    c.bench_function("encode_u64", |b| {
        b.iter(|| {
            let mut out = BytesMut::new();
            0x1023_4567_89ab_cdefu64.encode(&mut out);
        })
    });
    c.bench_function("encode_u256", |b| {
        b.iter(|| {
            let mut out = BytesMut::new();
            let uint = U256::from_be_bytes(hex!(
                "8090a0b0c0d0e0f00910203040506077000000000000000100000000000012f0"
            ));
            uint.encode(&mut out);
        })
    });
    c.bench_function("encode_1000_u64", |b| {
        b.iter(|| {
            let mut out = BytesMut::new();
            fastrlp::encode_list((0..1000u64).collect::<Vec<_>>().as_slice(), &mut out);
        })
    });
}

fn bench_decode(c: &mut Criterion) {
    c.bench_function("decode_u64", |b| {
        b.iter(|| {
            let data = [0x88, 0x10, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
            let _ = u64::decode(&mut &data[..]).unwrap();
        })
    });
    c.bench_function("decode_u256", |b| {
        b.iter(|| {
            let data = [
                0xa0, 0x80, 0x90, 0xa0, 0xb0, 0xc0, 0xd0, 0xe0, 0xf0, 0x09, 0x10, 0x20, 0x30, 0x40,
                0x50, 0x60, 0x77, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x12, 0xf0,
            ];
            let _ = U256::decode(&mut &data[..]).unwrap();
        })
    });
    c.bench_function("decode_1000_u64", |b| {
        let input = (0..1000u64).collect::<Vec<_>>();
        let mut data = BytesMut::new();
        fastrlp::encode_list(input.as_slice(), &mut data);
        b.iter(|| {
            let _ = Vec::<u64>::decode(&mut &data[..]).unwrap();
        });
    });
}

criterion_group!(benches, bench_encode, bench_decode);
criterion_main!(benches);
