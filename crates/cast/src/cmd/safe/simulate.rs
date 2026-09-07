use super::{
    contracts::{ISafe, ISimulateTxAccessor, SIMULATE_TX_ACCESSOR_V1_4_1},
    deploy::ensure_contract,
    rpc_provider,
    service::{SafeServiceOpts, SafeTransaction},
};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::{SolCall, SolValue};
use clap::Args;
use eyre::{Context, Result, ensure};
use foundry_cli::{json::print_json_object, opts::RpcOpts};
use serde_json::json;

/// CLI arguments for `cast safe simulate`.
#[derive(Args, Debug)]
pub struct SimulateArgs {
    /// Safe account address.
    safe: Address,

    /// Safe transaction hash from the Transaction Service.
    safe_tx_hash: B256,

    /// Address that will execute the Safe transaction. Used as the simulation's tx.origin.
    #[arg(long, env = "ETH_FROM", value_name = "ADDRESS")]
    from: Address,

    /// SimulateTxAccessor address.
    #[arg(long, default_value_t = SIMULATE_TX_ACCESSOR_V1_4_1)]
    accessor: Address,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    rpc: Box<RpcOpts>,
}

impl SimulateArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self { safe, safe_tx_hash, from, accessor, service, rpc } = self;
        let (provider, chain_id) = rpc_provider(&rpc).await?;
        let transaction = service.get_transaction(chain_id, "v2", safe_tx_hash).await?;
        transaction.verify_hash(safe, &provider).await?;
        transaction.show_transaction_summary()?;
        ensure!(
            SafeTransaction::number(&transaction.gas_price, "gasPrice")?.is_zero(),
            "cannot simulate reimbursed Safe transactions (gasPrice > 0): SimulateTxAccessor does not enforce safeTxGas"
        );
        ensure_contract(&provider, accessor, "SimulateTxAccessor", "--accessor").await?;

        let accessor_call = ISimulateTxAccessor::simulateCall {
            to: transaction.to,
            value: SafeTransaction::number(&transaction.value, "value")?,
            data: transaction.data.clone(),
            operation: transaction.operation,
        }
        .abi_encode();
        let simulation_call = ISafe::simulateAndRevertCall {
            targetContract: accessor,
            calldataPayload: accessor_call.into(),
        }
        .abi_encode();
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
        }))
    }
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
