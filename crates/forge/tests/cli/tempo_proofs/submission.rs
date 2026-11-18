//! Submit attached fork evidence as an application transaction on a separate local Tempo node.

use super::evidence::{self, Identity};
use alloy_dyn_abi::{DecodedEvent, DynSolValue, JsonAbiExt, Specifier};
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use anvil::{NodeConfig, eth::backend::tempo_consensus::TempoConsensus, spawn};
use foundry_compilers::artifacts::ConfigurableContractArtifact;
use serde_json::Value;

pub(super) async fn verify(
    config: NodeConfig,
    receiver: ConfigurableContractArtifact,
    fork_seed: B256,
    proof: &Value,
) {
    // Provision trust from the test's chosen seed, independently of the attached evidence.
    let epoch_length = config.tempo_epoch_length.expect("Tempo epoch length configured");
    let identity = TempoConsensus::new(fork_seed, epoch_length).identity();
    let key = evidence::key_coordinates(&identity.identity);
    let abi = receiver.abi.unwrap();
    let mut initcode = receiver.bytecode.unwrap().into_bytes().unwrap().to_vec();
    initcode.extend_from_slice(&key);

    let (_api, handle) =
        spawn(config.with_tempo_consensus_seed(Some(B256::repeat_byte(0x53)))).await;
    let provider = handle.http_provider();
    let destination_identity =
        provider.client().request::<_, Identity>("anvil_tempoConsensusIdentity", ()).await.unwrap();
    assert_ne!(destination_identity.identity, identity.identity);
    let senders = provider.get_accounts().await.unwrap();
    let deployment =
        TransactionRequest::default().from(senders[0]).transaction_type(2).with_input(initcode);
    // Tempo's code-deposit costs also apply to the verifier created by the receiver constructor.
    let gas = provider.estimate_gas(deployment.clone().into()).await.unwrap();
    let deployment = evidence::send(&provider, senders[0], deployment.gas_limit(gas)).await;
    assert_eq!(deployment.block_number(), Some(1));
    let address = deployment.contract_address().unwrap();

    let submit = &abi.functions["submit"][0];
    let attached = &proof["initial"]["evidence"];
    let height = DynSolValue::Uint(U256::from(proof["initial"]["height"].as_u64().unwrap()), 256);
    let account = DynSolValue::Address(serde_json::from_value(proof["account"].clone()).unwrap());
    let slot = DynSolValue::FixedBytes(B256::ZERO, 32);
    let value = DynSolValue::Uint(U256::from(42), 256);
    let mut args = vec![
        submit.inputs[0].resolve().unwrap().coerce_json(attached).unwrap(),
        height.clone(),
        account.clone(),
        slot.clone(),
        value.clone(),
    ];
    let submit_transaction = |args: &[DynSolValue]| {
        TransactionRequest::default()
            .from(senders[1])
            .transaction_type(2)
            .to(address)
            .with_input(submit.abi_encode_input(args).unwrap())
    };
    // A different account can relay the proof; trust is tied to the configured BLS key.
    let transaction = submit_transaction(&args);
    let gas = provider.estimate_gas(transaction.clone().into()).await.unwrap();
    let receipt = evidence::send(&provider, senders[1], transaction.gas_limit(gas)).await;
    assert_eq!(receipt.block_number(), Some(2));
    // Tempo receipts also contain fee-token logs, including on application reverts.
    let logs = receipt.logs().iter().filter(|log| log.address() == address).collect::<Vec<_>>();
    assert_eq!(logs.len(), 1);
    let log = logs[0];
    assert_eq!(log.transaction_hash, Some(receipt.transaction_hash()));
    let digest = serde_json::from_value(attached["subject"]["payload"].clone()).unwrap();
    let accepted = DecodedEvent {
        selector: Some(abi.events["ProofAccepted"][0].selector()),
        indexed: vec![DynSolValue::FixedBytes(digest, 32), height, account],
        body: vec![slot, value],
    };
    assert_eq!(log.data(), &accepted.encode_log_data());

    // Invalid claims are mined as reverted transactions and cannot publish acceptance logs.
    args[4] = DynSolValue::Uint(U256::from(43), 256);
    let transaction = submit_transaction(&args).gas_limit(gas);
    let error = provider.call(transaction.clone().into()).await.unwrap_err();
    assert_eq!(
        error.as_error_resp().unwrap().as_revert_data().unwrap().as_ref(),
        abi.errors["InvalidProof"][0].abi_encode_input(&[]).unwrap(),
    );
    let rejected = evidence::transact(&provider, senders[1], transaction).await;
    assert!(!rejected.status());
    assert_eq!(rejected.block_number(), Some(3));
    assert!(!rejected.logs().iter().any(|log| log.address() == address));
}
