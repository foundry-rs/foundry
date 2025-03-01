//! Disassemble evm bytecode into individual instructions.
//!
//! This crate provides a simple interface for disassembling evm bytecode into individual
//! instructions / opcodes.
//! It supports both hex encoded strings as well as a vector of bytes as input
//! Additionally it provides a method to format the disassembled instructions into a human readable
//! format identical to that of the [pyevmasm](https://github.com/crytic/pyevmasm) library
//!
//! ```rust
//! use evm_disassembler::{disassemble_str, disassemble_bytes, format_operations};
//!    
//! let bytecode = "60606040526040";
//! let instructions = disassemble_str(bytecode).unwrap();
//! // Will print:
//! // 00000000: PUSH1 0x60
//! // 00000002: PUSH1 0x40
//! // 00000004: MSTORE
//! // 00000005: PUSH1 0x40
//! println!("{}", format_operations(instructions).unwrap());
//!
//! let bytes = hex::decode(bytecode).unwrap();
//! let instructions_from_bytes = disassemble_bytes(bytes).unwrap();
//! println!("{}", format_operations(instructions_from_bytes).unwrap());
//!
//! ```
#![warn(missing_docs)]
use crate::decode::decode_operation;
use std::fmt::Write;

use eyre::Result;

mod decode;

pub mod types;
pub use types::{Opcode, Operation};

#[cfg(test)]
mod test_utils;

/// Disassemble a hex encoded string into a vector of instructions / operations
///
/// # Arguments
/// - `input` - A hex encoded string representing the bytecode to disassemble
///
/// # Examples
///
/// ```rust
/// use evm_disassembler::disassemble_str;
///
/// let bytecode = "0x608060405260043610603f57600035";
/// let instructions = disassemble_str(bytecode).unwrap();
/// ```
pub fn disassemble_str(input: &str) -> Result<Vec<Operation>> {
    let input = input.trim_start_matches("0x");
    let bytes = hex::decode(input)?;
    disassemble_bytes(bytes)
}

/// Disassemble a vector of bytes into a vector of decoded Operations
///
/// Will stop disassembling when it encounters a push instruction with a size greater than
/// remaining bytes in the input
///
/// # Arguments
/// - `bytes` - A vector of bytes representing the encoded bytecode
///
/// # Examples
///
/// ```rust
/// use evm_disassembler::disassemble_bytes;
///
/// let bytecode = "608060405260043610603f57600035";
/// let bytes = hex::decode(bytecode).unwrap();
/// let instructions_from_bytes = disassemble_bytes(bytes).unwrap();
/// ```
pub fn disassemble_bytes(bytes: Vec<u8>) -> Result<Vec<Operation>> {
    let mut operations = Vec::new();
    let mut new_operation: Operation;
    let mut offset = 0;
    let mut bytes_iter = bytes.into_iter();
    while bytes_iter.len() > 0 {
        (new_operation, offset) = match decode_operation(&mut bytes_iter, offset) {
            Ok((operation, new_offset)) => (operation, new_offset),
            Err(e) => {
                println!("Stop decoding at offset {offset} due to error : {e}");
                break;
            }
        };
        operations.push(new_operation);
    }
    Ok(operations)
}

/// Converts a vector of decoded operations into a human readable formatted string
///
/// Operations are formatted on individual lines with the following format:
/// `{offset}: {opcode} {bytes}`
///
/// - `offset` - The offset of the operation in the bytecode (as hex)
/// - `opcode` - The respective opcode (i.e. "PUSH1", "ADD")
/// - `bytes` - Additional bytes that are part of the operation (only for "PUSH" instructions)
///
/// # Arguments
/// - `operations` - A vector of decoded operations as returned by `disassemble_str` or
/// `disassemble_bytes`
///
/// # Examples
/// ```rust
/// use evm_disassembler::{disassemble_str, format_operations};
///
/// let bytecode = "0x608060405260043610603f57600035";
/// let instructions = disassemble_str(bytecode).unwrap();
/// println!("{}", format_operations(instructions).unwrap());
/// ```
pub fn format_operations(operations: Vec<Operation>) -> Result<String> {
    let mut formatted = String::new();
    for operation in operations.iter() {
        writeln!(formatted, "{operation:?}")?;
    }
    Ok(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::get_contract_code;
    use crate::types::Opcode;
    use rstest::*;
    use std::fs;

    #[rstest]
    #[case("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", 1577, vec![(Opcode::DUP7, 1000), (Opcode::EXTCODECOPY, 1563)])]
    #[tokio::test]
    async fn decode_code_from_rpc_provider(
        #[case] address: &str,
        #[case] expected_length: usize,
        #[case] expected_opcodes: Vec<(Opcode, usize)>,
    ) {
        let code = get_contract_code(address).await;
        let operations = disassemble_bytes(code).expect("Unable to disassemble code");
        assert_eq!(operations.len(), expected_length);
        for (opcode, expected_position) in expected_opcodes.iter() {
            assert_eq!(operations[*expected_position].opcode, *opcode);
        }
    }

    #[rstest]
    #[case("0xDef1C0ded9bec7F1a1670819833240f027b25EfF")] // UniswapV3 Router
    #[case("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")] // Weth
    #[case("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")] // ZeroEx Proxy
    #[case("0x00000000006c3852cbEf3e08E8dF289169EdE581")] // Seaport
    fn decode_code_from_file(#[case] address: &str) {
        let mut code = fs::read_to_string(format!("testdata/{address}_encoded.txt"))
            .expect("Unable to read encoded file");
        let decoded_reference = fs::read_to_string(format!("testdata/{address}_decoded.txt"))
            .expect("No reference file");
        code.pop();

        let operations = disassemble_str(&code).expect("Unable to decode");
        assert!(!operations.is_empty());
        let formatted_operations = format_operations(operations);
        for (i, line) in formatted_operations
            .expect("failed to format")
            .lines()
            .enumerate()
        {
            assert_eq!(line, decoded_reference.lines().nth(i).unwrap());
        }
        println!("Decoded output from contract {address} matches reference");
    }

    #[rstest]
    fn decode_preamble() {
        let code = "608060405260043610603f57600035";
        let operations = disassemble_str(code).expect("Unable to decode");
        assert_eq!(operations.len(), 10);
    }

    #[rstest]
    fn decode_preamble_from_bytes() {
        let bytes = hex::decode("608060405260043610603f57600035").unwrap();
        let operations = disassemble_bytes(bytes).expect("Unable to decode");
        assert_eq!(operations.len(), 10);
    }

    #[rstest]
    #[case(Opcode::STOP, "0x00")]
    #[case(Opcode::ADD, "0x01")]
    #[case(Opcode::MUL, "0x02")]
    #[case(Opcode::SUB, "0x03")]
    #[case(Opcode::DIV, "0x04")]
    #[case(Opcode::SDIV, "0x05")]
    #[case(Opcode::MOD, "0x06")]
    #[case(Opcode::SMOD, "0x07")]
    #[case(Opcode::ADDMOD, "0x08")]
    #[case(Opcode::MULMOD, "0x09")]
    fn decode_single_op(#[case] opcode: Opcode, #[case] encoded_opcode: &str) {
        let result = disassemble_str(encoded_opcode).expect("Unable to decode");
        assert_eq!(result, vec![Operation::new(opcode, 0)]);
    }

    #[rstest]
    fn decode_stop_and_add() {
        let add_op = "01";
        let stop_op = "00";
        let result = disassemble_str(&(add_op.to_owned() + stop_op)).expect("Unable to decode");
        assert_eq!(
            result,
            vec![
                Operation::new(Opcode::ADD, 0),
                Operation::new(Opcode::STOP, 1),
            ]
        );
    }
}
