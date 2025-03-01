use std::fs::File;
use std::io::{self, BufReader, Read};

use criterion::*;
use inferno::flamegraph::{self, Options};

fn flamegraph_benchmark(c: &mut Criterion, id: &str, infile: &str, mut opt: Options<'static>) {
    let mut f = File::open(infile).expect("file not found");

    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).expect("Could not read file");

    let mut group = c.benchmark_group(id);

    group
        .bench_with_input("flamegraph", &bytes, move |b, data| {
            b.iter(|| {
                let reader = BufReader::new(data.as_slice());
                let _folder = flamegraph::from_reader(&mut opt, reader, io::sink());
            })
        })
        .throughput(Throughput::Bytes(bytes.len() as u64));

    group.finish();
}

macro_rules! flamegraph_benchmarks {
    ($($name:ident : ($infile:expr, $opt:expr)),*) => {
        $(
            fn $name(c: &mut Criterion) {
                let id = stringify!($name);
                flamegraph_benchmark(c, id, $infile, $opt);
            }
        )*

        criterion_group!(benches, $($name),*);
        criterion_main!(benches);
    }
}

flamegraph_benchmarks! {
    flamegraph: ("tests/data/collapse-perf/results/example-perf-stacks-collapsed.txt",
                 { let mut opt = Options::default(); opt.reverse_stack_order = true; opt })
}
