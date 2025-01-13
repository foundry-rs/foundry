use alloy_consensus::Transaction;
use alloy_primitives::{hex, utils::format_units};
use forge_script_sequence::TransactionWithMetadata;
use foundry_common::TransactionMaybeSigned;
use std::fmt::{Error, Write};

/// Format transaction details for display
pub fn format_transaction_details(
    index: usize,
    tx: &TransactionWithMetadata,
) -> Result<String, Error> {
    let mut output = String::new();
    writeln!(output, "\n### Transaction {index} ###\n")?;

    // Contract info (to)
    if let Some(addr) = tx.contract_address {
        if let Some(name) = &tx.contract_name {
            writeln!(output, "to: {name}({addr})")?;
        } else {
            writeln!(output, "to: {addr}")?;
        }
    } else {
        writeln!(output, "to: <contract creation>")?;
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
            writeln!(output, "data: <empty>")?;
        } else {
            // Show decoded function if available
            if let (Some(func), Some(args)) = (&tx.function, &tx.arguments) {
                if args.is_empty() {
                    writeln!(output, "data (decoded): {func}()")?;
                } else {
                    writeln!(output, "data (decoded): {func}(")?;
                    for (i, arg) in args.iter().enumerate() {
                        writeln!(
                            &mut output,
                            "  {}{}",
                            arg,
                            if i + 1 < args.len() { "," } else { "" }
                        )?;
                    }
                    writeln!(output, ")")?;
                }
            }
            // Always show raw data
            writeln!(output, "data (raw): {}", hex::encode_prefixed(data))?;
        }
    }

    // Value
    if let Some(value) = value {
        let eth_value = format_units(value, 18).unwrap_or_else(|_| "N/A".into());

        writeln!(
            output,
            "value: {} wei [{} ETH]",
            value,
            eth_value.trim_end_matches('0').trim_end_matches('.')
        )?;
    }

    // Gas limit
    if let Some(gas) = gas {
        writeln!(output, "gasLimit: {gas}")?;
    }

    // Gas pricing
    match (max_fee_per_gas, max_priority_fee_per_gas, gas_price) {
        (Some(max_fee), Some(priority_fee), _) => {
            writeln!(output, "maxFeePerGas: {max_fee}")?;
            writeln!(output, "maxPriorityFeePerGas: {priority_fee}")?;
        }
        (_, _, Some(gas_price)) => {
            writeln!(output, "gasPrice: {gas_price}")?;
        }
        _ => {}
    }

    writeln!(output)?;
    Ok(output)
}
