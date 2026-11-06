//! Verify local development proofs after forking a native Tempo testnet anchor.

use super::{evidence, sources};
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{B256, Bytes, keccak256};
use alloy_provider::{Provider, ext::AnvilApi};
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest};
use anvil::{NodeConfig, eth::backend::tempo_consensus::TempoConsensus, spawn};
use foundry_common::{fs::read_json_file, provider::ProviderBuilder};
use foundry_compilers::artifacts::{
    ConfigurableContractArtifact, EvmVersion, remappings::Remapping,
};
use foundry_config::{
    SolcReq,
    fs_permissions::{FsPermissions, PathPermission},
};
use foundry_evm::hardfork::TempoHardfork;
use foundry_test_utils::str;
use serde_json::json;
use std::{num::NonZeroU64, time::Duration};
use tempo_alloy::{TempoNetwork, chainspec::TempoChainSpec, rpc::ForkSchedule};
use tokio::time::timeout;

#[forgetest]
#[ignore = "requires native Tempo testnet RPC"]
async fn tempo_proofs_native_fork(prj: _, cmd: _) {
    let url = std::env::var("TEMPO_TESTNET_RPC_URL")
        .expect("set TEMPO_TESTNET_RPC_URL to a native Tempo testnet RPC");
    assert!(!url.trim().is_empty(), "TEMPO_TESTNET_RPC_URL must not be empty");
    timeout(Duration::from_secs(180), async {
        let upstream = ProviderBuilder::<TempoNetwork>::new(&url).build().unwrap();
        let chain_id = upstream.get_chain_id().await.unwrap();
        let spec = TempoChainSpec::moderato();
        assert_eq!(chain_id, spec.inner.chain.id(), "source must be Moderato");
        let genesis = upstream.get_block_by_number(0.into()).await.unwrap().unwrap();
        assert_eq!(genesis.header.inner.hash, spec.inner.genesis_hash(), "source genesis");
        let anchor = upstream.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
        let anchor_header = anchor.header.as_ref();
        let anchor_height = anchor_header.inner.number;
        let anchor_hash = anchor.header.inner.hash;
        let schedule = upstream
            .raw_request::<_, ForkSchedule>("tempo_forkSchedule".into(), ())
            .await
            .expect("native source must expose tempo_forkSchedule");
        let hardfork = schedule
            .schedule
            .iter()
            .filter(|fork| fork.activation_time <= anchor_header.inner.timestamp)
            .map(|fork| fork.name.parse::<TempoHardfork>().expect("known source hardfork"))
            .max()
            .expect("source schedule must identify the anchor hardfork");
        let epoch = anchor_header.consensus_context.expect("native anchor consensus context").epoch;
        // Discover the current epoch boundary from native headers, rather than accepting Anvil's
        // fallback epoch length when the public source rejects its metadata RPC.
        let boundary_height = discover_epoch_boundary(anchor_height, epoch, async |height| {
            let block = upstream.get_block_by_number(height.into()).await.unwrap().unwrap();
            assert_eq!(block.header.as_ref().inner.number, height);
            block.header.as_ref().consensus_context.expect("native epoch context").epoch
        })
        .await;
        assert!(boundary_height > 0, "source epoch boundary must follow genesis");
        assert_eq!(boundary_height % epoch, 0, "source epochs must have a fixed length");
        let epoch_length = NonZeroU64::new(boundary_height / epoch).unwrap();
        assert_eq!(
            epoch_length,
            spec.info.epoch_length().expect("Moderato genesis defines epoch length"),
            "native epoch length must match the pinned chain specification",
        );
        assert_eq!(epoch, anchor_height / epoch_length);
        // Genesis hashes identify the chain but do not commit its epoch configuration. Check the
        // actual native transition on both sides of the observed boundary.
        let mut previous_hash = None;
        for (height, expected_epoch) in [(boundary_height - 1, epoch - 1), (boundary_height, epoch)]
        {
            let block = upstream.get_block_by_number(height.into()).await.unwrap().unwrap();
            assert_eq!(block.header.as_ref().inner.number, height);
            assert_eq!(
                block.header.as_ref().consensus_context.expect("native epoch context").epoch,
                expected_epoch,
                "source epoch at height {height}",
            );
            if let Some(hash) = previous_hash {
                assert_eq!(block.header.as_ref().inner.parent_hash, hash);
            }
            previous_hash = Some(block.header.inner.hash);
        }

        sources::prepare(&prj).await;
        prj.insert_vm();
        prj.add_raw_source(
            "TempoProofVerifier.sol",
            include_str!("../../../../../testdata/fixtures/TempoProofVerifier.sol"),
        );
        prj.add_raw_test(
            "TempoNativeProof.t.sol",
            include_str!("../../../../../testdata/fixtures/TempoNativeProof.t.sol"),
        );
        prj.update_config(|config| {
            config.solc = Some(SolcReq::Version("0.8.30".parse().unwrap()));
            config.evm_version = EvmVersion::Prague;
            config.optimizer = Some(true);
            config.via_ir = true;
            config.fs_permissions = FsPermissions::new([PathPermission::read("fixtures")]);
            config.remappings =
                ["commonware/=lib/commonware/src/", "src/libraries/=lib/optimism/src/libraries/"]
                    .map(|remapping| remapping.parse::<Remapping>().unwrap().into())
                    .into();
        });
        cmd.args(["build"]).assert_success();
        let store = read_json_file::<ConfigurableContractArtifact>(
            &prj.artifacts().join("TempoProofVerifier.sol/ProofStore.json"),
        )
        .unwrap();
        let bytecode = store.bytecode.unwrap().into_bytes().unwrap();
        let seed = B256::repeat_byte(0x54);
        let identity = TempoConsensus::new(seed, epoch_length).identity();
        // Configure only the strictly sourced epoch length; hardfork inference stays automatic.
        let config = NodeConfig::test_tempo()
            .with_eth_rpc_url(Some(url.clone()))
            .with_fork_block_number(Some(anchor_height))
            .with_no_storage_caching(true)
            .with_no_mining(true)
            .with_tempo_consensus_seed(Some(seed))
            .with_tempo_epoch_length(Some(epoch_length));
        assert!(config.hardfork.is_none());
        let (_api, handle) = spawn(config).await;
        let provider = handle.http_provider();
        let info = provider.anvil_node_info().await.unwrap();
        assert_eq!(info.hard_fork.parse::<TempoHardfork>().unwrap(), hardfork);
        assert_eq!(provider.get_chain_id().await.unwrap(), chain_id);
        assert_eq!(provider.get_block_number().await.unwrap(), anchor_height);
        assert_eq!(
            provider.get_block_by_number(anchor_height.into()).await.unwrap().unwrap().header.hash,
            anchor_hash,
        );
        let local_identity = provider
            .raw_request::<_, serde_json::Value>("anvil_tempoConsensusIdentity".into(), ())
            .await
            .unwrap();
        assert_eq!(local_identity["identity"], json!(identity.identity));
        assert_eq!(local_identity["epochLength"], json!(epoch_length.get()));
        let sender = provider.get_accounts().await.unwrap()[0];
        let deployment =
            evidence::send(&provider, sender, TransactionRequest::default().with_input(bytecode))
                .await;
        assert_eq!(deployment.block_number(), Some(anchor_height + 1));
        let account = deployment.contract_address().unwrap();
        let receipt = evidence::send(&provider, sender, evidence::write(account, 42)).await;
        let local_height = receipt.block_number().unwrap();
        assert_eq!(local_height, anchor_height + 2);
        let local = evidence::record(&provider, local_height, Some(account)).await;
        let raw_header =
            serde_json::from_value::<Bytes>(local.evidence["rawHeader"].clone()).unwrap();
        assert_eq!(json!(keccak256(raw_header)), local.evidence["subject"]["payload"]);
        let proof = json!({
            "anchor": { "height": anchor_height, "hash": anchor_hash },
            "local": local,
            "account": account,
            "trustedKey": evidence::key_coordinates(&identity.identity),
        });
        prj.create_file(
            "fixtures/native-proof.json",
            &serde_json::to_string_pretty(&proof).unwrap(),
        );
        cmd.forge_fuse().args(["test", "--offline"]).assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/TempoNativeProof.t.sol:TempoNativeProofTest
[PASS] test_native_fork_certified_storage_proof() ([GAS])
[PASS] test_native_fork_rejects_wrong_value() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
    })
    .await
    .expect("verify native Tempo fork proofs within 180 seconds");
}

/// Finds the epoch boundary by searching fixed lengths compatible with the source anchor.
async fn discover_epoch_boundary(
    anchor_height: u64,
    epoch: u64,
    mut epoch_at: impl AsyncFnMut(u64) -> u64,
) -> u64 {
    assert!(epoch > 0, "source must have crossed an epoch boundary");
    assert!(epoch <= anchor_height, "source epoch exceeds its anchor height");
    // For fixed epochs, epoch = floor(anchor_height / length), so search only lengths in
    // (anchor_height / (epoch + 1), anchor_height / epoch] rather than every source height.
    let next_epoch = epoch.checked_add(1).expect("source epoch must have a successor");
    let mut low = anchor_height / next_epoch + 1;
    let mut high = anchor_height / epoch;
    assert!(low <= high, "source anchor has no fixed epoch-length candidate");
    while low < high {
        let candidate = low + (high - low) / 2;
        let height =
            epoch.checked_mul(candidate).expect("epoch boundary must fit in source height");
        let observed = epoch_at(height).await;
        if observed < epoch {
            low = candidate + 1;
        } else {
            assert_eq!(observed, epoch, "source epoch at height {height}");
            high = candidate;
        }
    }
    epoch.checked_mul(low).expect("epoch boundary must fit in source height")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn epoch_search_limits_rpc_queries() {
        // At this native benchmark anchor, the original height search needed 25 queries.
        let anchor_height = 42_863_316;
        let epoch_length = 21_600;
        let epoch = anchor_height / epoch_length;
        let mut queries = 0;
        let boundary = discover_epoch_boundary(anchor_height, epoch, async |height| {
            queries += 1;
            assert!(height <= anchor_height);
            height / epoch_length
        })
        .await;
        assert_eq!(boundary, 42_854_400);
        assert!(queries <= 4, "expected at most four epoch queries, got {queries}");
    }

    #[tokio::test]
    async fn epoch_search_handles_epoch_edges() {
        for length in [2, 3, 4, 7, 21_600] {
            for epoch in [1, 2, 17, 1984] {
                let boundary = epoch * length;
                for anchor in [boundary, boundary + length - 1] {
                    let actual = discover_epoch_boundary(anchor, epoch, async |height| {
                        assert!(height <= anchor);
                        height / length
                    })
                    .await;
                    assert_eq!(actual, boundary, "length {length}, epoch {epoch}, anchor {anchor}");
                }
            }
        }
    }

    #[tokio::test]
    async fn epoch_search_skips_queries_for_single_candidate() {
        let boundary = discover_epoch_boundary(101, 50, async |_| {
            unreachable!("the anchor determines the only candidate length")
        })
        .await;
        assert_eq!(boundary, 100);
    }

    #[tokio::test]
    async fn epoch_search_handles_large_heights() {
        for (length, anchor) in [
            (2, u64::MAX),
            (3, u64::MAX - 1),
            (u64::MAX / 3, u64::MAX),
            (u64::MAX / 2 + 1, u64::MAX),
        ] {
            let epoch = anchor / length;
            let mut queries = 0;
            let boundary = discover_epoch_boundary(anchor, epoch, async |height| {
                queries += 1;
                assert!(height <= anchor);
                height / length
            })
            .await;
            assert_eq!(boundary, epoch * length);
            assert!(queries <= u64::BITS);
        }
    }

    #[tokio::test]
    #[should_panic(expected = "source must have crossed an epoch boundary")]
    async fn epoch_search_rejects_epoch_zero() {
        discover_epoch_boundary(100, 0, async |_| unreachable!()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "source epoch exceeds its anchor height")]
    async fn epoch_search_rejects_epoch_above_height() {
        discover_epoch_boundary(5, 6, async |_| unreachable!()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "source anchor has no fixed epoch-length candidate")]
    async fn epoch_search_rejects_inconsistent_anchor() {
        discover_epoch_boundary(7, 5, async |_| unreachable!()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "source epoch must have a successor")]
    async fn epoch_search_rejects_epoch_overflow() {
        discover_epoch_boundary(u64::MAX, u64::MAX, async |_| unreachable!()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "source epoch at height")]
    async fn epoch_search_rejects_epoch_after_anchor() {
        discover_epoch_boundary(42_863_316, 1984, async |_| 1985).await;
    }
}
