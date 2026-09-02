use super::{
    contracts::{ISafe, ISimulateTxAccessor},
    deploy::ensure_contract,
    service::{SafeServiceOpts, SafeTransaction},
};
use alloy_network::{Ethereum, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::{SolCall, SolValue};
use eyre::{Context, Result, ensure};
use foundry_cli::{json::print_json_object, opts::RpcOpts, utils::LoadConfig};
use foundry_common::provider::ProviderBuilder;
use serde_json::json;

pub(super) async fn run(
    safe: Address,
    safe_tx_hash: B256,
    from: Address,
    accessor: Address,
    service: SafeServiceOpts,
    rpc: RpcOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    let transaction = service.get_transaction(chain_id, "v2", safe_tx_hash).await?;
    transaction.verify_hash(safe, &provider).await?;
    transaction.show_transaction_summary()?;
    ensure!(
        SafeTransaction::number(&transaction.gas_price, "gasPrice")?.is_zero(),
        "cannot simulate reimbursed Safe transactions (gasPrice > 0): SimulateTxAccessor does not enforce safeTxGas"
    );
    ensure_contract(&provider, accessor, "SimulateTxAccessor", "--accessor").await?;

    let accessor_call: Bytes = ISimulateTxAccessor::simulateCall {
        to: transaction.to,
        value: SafeTransaction::number(&transaction.value, "value")?,
        data: transaction.data.clone(),
        operation: transaction.operation,
    }
    .abi_encode()
    .into();
    let simulation_call: Bytes =
        ISafe::simulateAndRevertCall { targetContract: accessor, calldataPayload: accessor_call }
            .abi_encode()
            .into();
    let request =
        TransactionRequest::default().with_from(from).with_to(safe).with_input(simulation_call);
    let Err(error) = provider.call(request).await else {
        eyre::bail!("Safe simulateAndRevert unexpectedly returned successfully");
    };
    let revert_data = error
        .as_error_resp()
        .and_then(|payload| payload.as_revert_data())
        .ok_or_else(|| eyre::eyre!("Safe simulation failed without revert data: {error}"))?;
    let (gas_used, success, return_data) = decode_result(&revert_data)?;
    ensure!(
        success,
        "Safe transaction simulation failed after {gas_used} gas; return data: {return_data}"
    );
    print_json_object(json!({
        "safeTxHash": safe_tx_hash,
        "success": true,
        "gasUsed": gas_used.to_string(),
        "returnData": return_data,
    }))?;
    Ok(())
}

fn decode_result(revert_data: &[u8]) -> Result<(U256, bool, Bytes)> {
    let header_length = 2 * U256::BYTES;
    ensure!(revert_data.len() >= header_length, "invalid Safe simulateAndRevert response");
    let accessor_success = !U256::from_be_slice(&revert_data[..U256::BYTES]).is_zero();
    let response_len = U256::from_be_slice(&revert_data[U256::BYTES..header_length]);
    let available = revert_data.len() - header_length;
    ensure!(
        response_len <= U256::from(available),
        "invalid Safe simulateAndRevert response length"
    );
    let response = &revert_data[header_length..header_length + response_len.to::<usize>()];
    ensure!(
        accessor_success,
        "SimulateTxAccessor delegatecall failed; return data: {}",
        hex::encode_prefixed(response)
    );
    <(U256, bool, Bytes)>::abi_decode_params(response)
        .wrap_err("invalid SimulateTxAccessor response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_signature_independent_simulation_result() {
        let return_data = Bytes::from_static(b"result");
        let accessor_response = (U256::from(42), true, return_data.clone()).abi_encode_params();
        let mut revert_data = Vec::with_capacity(2 * U256::BYTES + accessor_response.len());
        revert_data.extend_from_slice(&U256::from(1).to_be_bytes::<{ U256::BYTES }>());
        revert_data.extend_from_slice(
            &U256::from(accessor_response.len()).to_be_bytes::<{ U256::BYTES }>(),
        );
        revert_data.extend_from_slice(&accessor_response);

        let decoded = decode_result(&revert_data).unwrap();
        assert_eq!(decoded, (U256::from(42), true, return_data));
    }
}
