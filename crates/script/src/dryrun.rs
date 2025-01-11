use alloy_consensus::Transaction;
use alloy_primitives::{hex, utils::format_units};
use forge_script_sequence::TransactionWithMetadata;
use foundry_common::TransactionMaybeSigned;

/// Format transaction details for display
pub fn format_transaction_details(index: usize, tx: &TransactionWithMetadata) -> String {
    let mut output = format!("\n### Transaction {index} ###\n\n");

    // Contract info (to)
    if let Some(addr) = tx.contract_address {
        if let Some(name) = &tx.contract_name {
            output.push_str(&format!("to: {name}({addr})\n"));
        } else {
            output.push_str(&format!("to: {addr}\n"));
        }
    } else {
        output.push_str("to: <contract creation>\n");
    }

    // Transaction data
    let (input, gas, gas_price, value, max_fee_per_gas, max_priority_fee_per_gas) = match tx.tx() {
        TransactionMaybeSigned::Signed { tx, .. } => (
            Some(tx.input()),
            Some(tx.gas_limit()),
            tx.gas_price(),
            Some(tx.value()),
            Some(tx.max_fee_per_gas()),
            tx.max_priority_fee_per_gas(),
        ),
        TransactionMaybeSigned::Unsigned(tx) => (
            tx.input.input(),
            tx.gas,
            tx.gas_price,
            tx.value,
            tx.max_fee_per_gas,
            tx.max_priority_fee_per_gas,
        ),
    };

    // Data field
    if let Some(data) = input {
        if data.is_empty() {
            output.push_str("data: <empty>\n");
        } else {
            // Show decoded function if available
            if let (Some(func), Some(args)) = (&tx.function, &tx.arguments) {
                if args.is_empty() {
                    output.push_str(&format!("data (decoded): {func}()\n"));
                } else {
                    output.push_str(&format!("data (decoded): {func}(\n"));
                    for (i, arg) in args.iter().enumerate() {
                        output.push_str(&format!(
                            "  {}{}\n",
                            arg,
                            if i + 1 < args.len() { "," } else { "" }
                        ));
                    }
                    output.push_str(")\n");
                }
            }
            // Always show raw data
            output.push_str(&format!("data (raw): {}\n", hex::encode_prefixed(data)));
        }
    }

    // Value
    if let Some(value) = value {
        let eth_value = format_units(value, 18).unwrap_or_else(|_| "N/A".into());
        output.push_str(&format!(
            "value: {} wei [{} ETH]\n",
            value,
            eth_value.trim_end_matches('0').trim_end_matches('.')
        ));
    }

    // Gas limit
    if let Some(gas) = gas {
        output.push_str(&format!("gasLimit: {gas}\n"));
    }

    // Gas pricing
    match (max_fee_per_gas, max_priority_fee_per_gas, gas_price) {
        (Some(max_fee), Some(priority_fee), _) => {
            output.push_str(&format!("maxFeePerGas: {max_fee}\n"));
            output.push_str(&format!("maxPriorityFeePerGas: {priority_fee}\n"));
        }
        (_, _, Some(gas_price)) => {
            output.push_str(&format!("gasPrice: {gas_price}\n"));
        }
        _ => {}
    }

    output.push('\n');
    output
}
