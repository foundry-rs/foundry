use forge_script_sequence::TransactionWithMetadata;
use foundry_common::fmt::UIfmt;
use std::fmt::{Error, Write};

/// Format transaction details for display
pub fn format_transaction_details(
    index: usize,
    tx: &TransactionWithMetadata,
) -> Result<String, Error> {
    let mut output = String::new();
    writeln!(output, "\n### Transaction {index} ###\n")?;
    writeln!(output, "{}", tx.tx().pretty())?;

    // Show contract name and address if available
    if !tx.opcode.is_any_create() {
        if let (Some(name), Some(addr)) = (&tx.contract_name, &tx.contract_address) {
            writeln!(output, "contract: {name}({addr})")?;
        }
    }

    // Show decoded function if available
    if let (Some(func), Some(args)) = (&tx.function, &tx.arguments) {
        if args.is_empty() {
            writeln!(output, "data (decoded): {func}()")?;
        } else {
            writeln!(output, "data (decoded): {func}(")?;
            for (i, arg) in args.iter().enumerate() {
                writeln!(&mut output, "  {}{}", arg, if i + 1 < args.len() { "," } else { "" })?;
            }
            writeln!(output, ")")?;
        }
    }

    writeln!(output)?;
    Ok(output)
}
