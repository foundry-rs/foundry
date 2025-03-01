use std::fs::File;
use std::io::{self, Read};

use criterion::*;
use inferno::collapse::{dtrace, perf, sample, Collapse};
use libflate::gzip::Decoder;
use once_cell::sync::Lazy;

const INFILE_DTRACE: &str = "flamegraph/example-dtrace-stacks.txt";
const INFILE_PERF: &str = "flamegraph/example-perf-stacks.txt.gz";
const INFILE_SAMPLE: &str = "tests/data/collapse-sample/large.txt.gz";
const SAMPLE_SIZE: usize = 100;

static NTHREADS: Lazy<usize> = Lazy::new(|| std::thread::available_parallelism().unwrap().into());

fn read_infile(infile: &str, buf: &mut Vec<u8>) -> io::Result<()> {
    let mut f = File::open(infile)?;
    if infile.ends_with(".gz") {
        let mut r = io::BufReader::new(Decoder::new(f)?);
        r.read_to_end(buf)?;
    } else {
        f.read_to_end(buf)?;
    }
    Ok(())
}

macro_rules! benchmark_single {
    ($name:ident, $name_str:expr, $infile:expr) => {
        fn $name(c: &mut Criterion) {
            let mut bytes = Vec::new();
            read_infile($infile, &mut bytes).unwrap();

            let mut collapser = $name::Folder::default();

            let mut group = c.benchmark_group($name_str);

            group
                .bench_with_input("collapse", &bytes, move |b, data| {
                    b.iter(|| {
                        let _result = collapser.collapse(data.as_slice(), io::sink());
                    })
                })
                .throughput(Throughput::Bytes(bytes.len() as u64))
                .sample_size(SAMPLE_SIZE);

            group.finish();
        }
    };
}

macro_rules! benchmark_multi {
    ($name:ident, $name_str:expr, $infile:expr) => {
        fn $name(c: &mut Criterion) {
            let mut bytes = Vec::new();
            read_infile($infile, &mut bytes).unwrap();

            let mut collapser1 = {
                let mut options = $name::Options::default();
                options.nthreads = 1;
                $name::Folder::from(options)
            };

            let mut collapser2 = {
                let mut options = $name::Options::default();
                options.nthreads = *NTHREADS;
                $name::Folder::from(options)
            };

            let mut group = c.benchmark_group("collapse");

            group
                .bench_with_input(format!("{}/{}", $name_str, 1), &bytes, move |b, data| {
                    b.iter(|| {
                        let _result = collapser1.collapse(data.as_slice(), io::sink());
                    })
                })
                .throughput(Throughput::Bytes(bytes.len() as u64))
                .sample_size(SAMPLE_SIZE);

            group
                .bench_with_input(
                    format!("{}/{}", $name_str, *NTHREADS),
                    &bytes,
                    move |b, data| {
                        b.iter(|| {
                            let _result = collapser2.collapse(data.as_slice(), io::sink());
                        })
                    },
                )
                .throughput(Throughput::Bytes(bytes.len() as u64))
                .sample_size(SAMPLE_SIZE);

            group.finish();
        }
    };
}

benchmark_multi!(dtrace, "dtrace", INFILE_DTRACE);
benchmark_multi!(perf, "perf", INFILE_PERF);
benchmark_single!(sample, "sample", INFILE_SAMPLE);

criterion_group!(benches, dtrace, perf, sample);

criterion_main!(benches);
