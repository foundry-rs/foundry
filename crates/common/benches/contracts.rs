use alloy_json_abi::JsonAbi;
use alloy_primitives::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use foundry_common::contracts::ContractsByArtifact;
use foundry_compilers::{
    ArtifactId,
    artifacts::{
        BytecodeObject, CompactBytecode, CompactContractBytecode, CompactDeployedBytecode,
    },
};
use std::hint::black_box;

const ARTIFACT_COUNTS: [usize; 3] = [100, 1_000, 5_000];

fn artifact_id(index: usize) -> ArtifactId {
    let name = format!("Contract{index:05}");
    ArtifactId {
        path: format!("out/{name}.sol/{name}.json").into(),
        name: name.clone(),
        source: format!("src/{name}.sol").into(),
        version: "0.8.30".parse().unwrap(),
        build_id: "benchmark".to_owned(),
        profile: "default".to_owned(),
    }
}

fn bytecode(index: usize) -> Bytes {
    let mut code = vec![0x60; 512 + index % 128];
    code[..8].copy_from_slice(&(index as u64).to_be_bytes());
    let len = code.len();
    code[len - 2..].fill(0xff);
    code.into()
}

fn contracts(count: usize) -> (ContractsByArtifact, Vec<Bytes>) {
    let mut codes = Vec::with_capacity(count);
    let artifacts = (0..count).map(|index| {
        let code = bytecode(index);
        codes.push(code.clone());
        let deployed_bytecode = CompactDeployedBytecode {
            bytecode: Some(CompactBytecode {
                object: BytecodeObject::Bytecode(code),
                source_map: None,
                link_references: Default::default(),
            }),
            immutable_references: Default::default(),
        };
        let artifact = CompactContractBytecode {
            abi: Some(JsonAbi::new()),
            bytecode: None,
            deployed_bytecode: Some(deployed_bytecode),
        };
        (artifact_id(index), artifact)
    });
    (ContractsByArtifact::new(artifacts), codes)
}

fn bench_exact_matches(c: &mut Criterion) {
    for count in ARTIFACT_COUNTS {
        let (contracts, codes) = contracts(count);
        let queries = [
            ("first", codes[0].clone()),
            ("middle", codes[count / 2].clone()),
            ("last", codes[count - 1].clone()),
            ("miss", Bytes::from(vec![0xfe; 777])),
        ];

        let mut group = c.benchmark_group(format!("exact_match/{count}"));
        group.throughput(Throughput::Elements(count as u64));
        for (position, query) in queries {
            group.bench_with_input(BenchmarkId::from_parameter(position), &query, |b, query| {
                b.iter(|| black_box(contracts.find_by_deployed_code_exact(black_box(query))));
            });
        }
        group.finish();
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(50).warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_exact_matches
);
criterion_main!(benches);
