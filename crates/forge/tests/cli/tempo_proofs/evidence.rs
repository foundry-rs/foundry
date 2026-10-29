//! Collect fresh Tempo HTTP evidence without verifying it on the producer side.

use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_network::{AnyTransactionReceipt, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::{
    Provider,
    ext::{AnvilApi, DebugApi},
};
use alloy_rpc_types::{BlockId, TransactionRequest};
use anvil::eth::backend::tempo_consensus::TempoFinalization;
use blst::min_sig::{PublicKey, Signature};
use commonware_codec::{ReadExt, varint::UInt};
use foundry_common::provider::RetryProvider;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    identity: Bytes,
    epoch_length: u64,
}

#[derive(Serialize)]
struct Record {
    height: u64,
    evidence: Value,
}

pub(super) async fn collect(
    provider: &RetryProvider,
    bytecode: Bytes,
    deployed_bytecode: Bytes,
) -> Value {
    let identity =
        provider.client().request::<_, Identity>("anvil_tempoConsensusIdentity", ()).await.unwrap();
    assert_eq!(identity.epoch_length, 4);
    let sender = provider.get_accounts().await.unwrap()[0];
    let deployment =
        send(provider, sender, TransactionRequest::default().with_input(bytecode)).await;
    let account = deployment.contract_address().unwrap();
    let receipt = send(provider, sender, write(account, 42)).await;
    assert_eq!(deployment.block_number(), Some(1));
    assert_eq!(receipt.block_number(), Some(2));
    assert_eq!(provider.get_code_at(account).number(2).await.unwrap(), deployed_bytecode);
    let initial = record(provider, 2, Some(account)).await;

    // Collect ordinary evidence before scheduling the authenticated key handoff.
    let next_identity = provider
        .client()
        .request::<_, Identity>("anvil_tempoRotateConsensusIdentity", ())
        .await
        .unwrap();
    mine(provider).await;
    let boundary = record(provider, 3, None).await;
    mine(provider).await;
    let successor = record(provider, 4, Some(account)).await;
    json!({
        "initialKey": key_coordinates(&identity.identity),
        "nextKey": key_coordinates(&next_identity.identity),
        "account": account,
        "initial": initial,
        "boundary": boundary,
        "successor": successor,
    })
}

async fn record(provider: &RetryProvider, height: u64, account: Option<Address>) -> Record {
    let finalization = provider
        .client()
        .request::<_, TempoFinalization>(
            "consensus_getFinalization",
            (json!({ "height": height }),),
        )
        .await
        .unwrap();
    let raw_header = provider.debug_get_raw_header(BlockId::number(height)).await.unwrap();
    let block = provider.get_block_by_number(height.into()).await.unwrap().unwrap();
    assert_eq!(block.header.hash, finalization.digest);
    let mut cursor = finalization.certificate.as_ref();
    let epoch = UInt::<u64>::read(&mut cursor).unwrap().0;
    let view = UInt::<u64>::read(&mut cursor).unwrap().0;
    let parent = UInt::<u64>::read(&mut cursor).unwrap().0;
    let (payload, signatures) = cursor.split_at(32);
    assert_eq!(B256::from_slice(payload), finalization.digest);
    assert_eq!((epoch, view), (finalization.epoch, finalization.view));
    assert_eq!(signatures.len(), 96);
    let (vote, seed) = signatures.split_at(48);
    let mut evidence = json!({
        "rawHeader": raw_header,
        "subject": { "kind": 2, "epoch": epoch, "viewNumber": view, "parent": parent, "payload": finalization.digest },
        "vote": Bytes::from(Signature::sig_validate(vote, true).unwrap().serialize()),
        "seed": Bytes::from(Signature::sig_validate(seed, true).unwrap().serialize()),
        "accountProof": [],
        "storageProof": [],
    });
    if let Some(account) = account {
        let proof = provider.get_proof(account, vec![B256::ZERO]).number(height).await.unwrap();
        evidence["accountProof"] = json!(proof.account_proof);
        evidence["storageProof"] = json!(proof.storage_proof[0].proof);
    }
    Record { height, evidence }
}

async fn send(
    provider: &RetryProvider,
    sender: Address,
    transaction: TransactionRequest,
) -> AnyTransactionReceipt {
    let hash = provider
        .client()
        .request::<_, B256>(
            "eth_sendTransaction",
            (transaction.from(sender).transaction_type(2).gas_limit(5_000_000),),
        )
        .await
        .unwrap();
    mine(provider).await;
    let receipt = provider.get_transaction_receipt(hash).await.unwrap().unwrap();
    assert!(receipt.status());
    receipt
}

async fn mine(provider: &RetryProvider) {
    provider.anvil_mine(Some(1), None).await.unwrap();
}

fn write(account: Address, value: u64) -> TransactionRequest {
    let setter = Function::parse("set(uint256)").unwrap();
    let input = setter.abi_encode_input(&[DynSolValue::Uint(U256::from(value), 256)]).unwrap();
    TransactionRequest::default().to(account).with_input(input)
}

fn key_coordinates(compressed: &[u8]) -> Bytes {
    let encoded = PublicKey::key_validate(compressed).unwrap().serialize();
    let mut output = [0; 256];
    // blst puts each Fp2 imaginary component first; EIP-2537 uses padded real components first.
    for (destination, source) in output.as_chunks_mut::<64>().0.iter_mut().zip([1, 0, 3, 2]) {
        destination[16..].copy_from_slice(&encoded[source * 48..(source + 1) * 48]);
    }
    output.into()
}
