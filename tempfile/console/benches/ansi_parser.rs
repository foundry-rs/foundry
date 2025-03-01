use console::{strip_ansi_codes, AnsiCodeIterator};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use std::{fs, path::Path};

pub fn parse_throughput(c: &mut Criterion) {
    let session_log_path = Path::new("tests")
        .join("data")
        .join("sample_zellij_session.log");
    let session_log = fs::read_to_string(session_log_path).unwrap();

    let mut group = c.benchmark_group("ansi-parsing");
    group.throughput(Throughput::Bytes(session_log.len() as u64));
    group.bench_function("parse", |b| {
        b.iter(|| {
            let v: Vec<_> = AnsiCodeIterator::new(&session_log).collect();
            black_box(v);
        })
    });
    group.bench_function("strip", |b| {
        b.iter(|| black_box(strip_ansi_codes(&session_log)))
    });
    group.finish();
}

criterion_group!(throughput, parse_throughput);
criterion_main!(throughput);
