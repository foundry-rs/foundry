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
        assert!(epoch > 0, "source must have crossed an epoch boundary");
        let (mut low, mut high) = (0, anchor_height);
        while low < high {
            let height = low + (high - low) / 2;
            let block = upstream.get_block_by_number(height.into()).await.unwrap().unwrap();
            assert_eq!(block.header.as_ref().inner.number, height);
            let observed =
                block.header.as_ref().consensus_context.expect("native epoch context").epoch;
            if observed < epoch {
                low = height + 1;
            } else {
                assert_eq!(observed, epoch, "source epoch at height {height}");
                high = height;
            }
        }
        let boundary_height = low;
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
