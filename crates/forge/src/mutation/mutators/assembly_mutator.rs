//! Assembly (Yul) block mutator.
//!
//! Mutates opcodes within inline assembly blocks by swapping them with
//! semantically related alternatives to test coverage of assembly code.
//!
//! # Mutation Categories
//!
//! ## Arithmetic Operations
//! - `add` ↔ `sub`
//! - `mul` ↔ `div`
//! - `mod` ↔ `div`
//! - `exp` → `mul`
//!
//! ## Comparison Operations
//! - `lt` ↔ `gt`
//! - `slt` ↔ `sgt`
//! - `eq` → `iszero(sub(a, b))`
//!
//! ## Bitwise Operations
//! - `and` ↔ `or`
//! - `xor` → `and`, `or`
//! - `not` → identity (remove not)
//! - `shl` ↔ `shr`
//! - `sar` ↔ `shr`

use std::collections::HashMap;

use eyre::Result;
use solar::ast::yul;

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

/// Mutator for Yul/assembly opcodes.
pub struct AssemblyMutator {
    /// Maps opcodes to their mutation alternatives.
    opcode_mutations: HashMap<&'static str, Vec<&'static str>>,
}

impl Default for AssemblyMutator {
    fn default() -> Self {
        Self::new()
    }
}

impl AssemblyMutator {
    pub fn new() -> Self {
        let mut opcode_mutations: HashMap<&'static str, Vec<&'static str>> = HashMap::new();

        // Arithmetic operations
        opcode_mutations.insert("add", vec!["sub", "mul", "xor"]);
        opcode_mutations.insert("sub", vec!["add", "mul", "div"]);
        opcode_mutations.insert("mul", vec!["add", "div", "and"]);
        opcode_mutations.insert("div", vec!["mul", "sub", "mod"]);
        opcode_mutations.insert("sdiv", vec!["smod", "mul"]);
        opcode_mutations.insert("mod", vec!["div", "mul"]);
        opcode_mutations.insert("smod", vec!["sdiv", "mod"]);
        opcode_mutations.insert("exp", vec!["mul", "add"]);
        opcode_mutations.insert("addmod", vec!["mulmod"]);
        opcode_mutations.insert("mulmod", vec!["addmod"]);

        // Comparison operations
        opcode_mutations.insert("lt", vec!["gt", "eq", "slt"]);
        opcode_mutations.insert("gt", vec!["lt", "eq", "sgt"]);
        opcode_mutations.insert("slt", vec!["sgt", "lt"]);
        opcode_mutations.insert("sgt", vec!["slt", "gt"]);
        opcode_mutations.insert("eq", vec!["lt", "gt"]);
        opcode_mutations.insert("iszero", vec!["not"]);

        // Bitwise operations
        opcode_mutations.insert("and", vec!["or", "xor"]);
        opcode_mutations.insert("or", vec!["and", "xor"]);
        opcode_mutations.insert("xor", vec!["and", "or"]);
        opcode_mutations.insert("not", vec!["iszero"]);
        opcode_mutations.insert("shl", vec!["shr", "sar"]);
        opcode_mutations.insert("shr", vec!["shl", "sar"]);
        opcode_mutations.insert("sar", vec!["shr", "shl"]);

        // Memory operations (dangerous but useful for testing)
        opcode_mutations.insert("mload", vec!["sload"]);
        opcode_mutations.insert("sload", vec!["mload"]);

        Self { opcode_mutations }
    }

    /// Get mutations for a given opcode name.
    pub fn get_mutations(&self, opcode: &str) -> Option<&Vec<&'static str>> {
        self.opcode_mutations.get(opcode)
    }
}

impl Mutator for AssemblyMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let yul_expr = context.yul_expr.ok_or_else(|| eyre::eyre!("No Yul expression"))?;

        // Only handle function calls (opcodes)
        let call = match &yul_expr.kind {
            yul::ExprKind::Call(call) => call,
            _ => return Ok(vec![]),
        };

        let opcode_name = call.name.as_str();

        // Get mutation alternatives for this opcode
        let alternatives = match self.get_mutations(opcode_name) {
            Some(alts) => alts,
            None => return Ok(vec![]),
        };

        // Extract original text and build mutants
        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();
        let column_number = context.column_number();

        let mutants = alternatives
            .iter()
            .map(|&new_opcode| {
                let mutated = mutate_yul_call(&original, opcode_name, new_opcode);
                Mutant {
                    span: context.span,
                    mutation: MutationType::YulOpcode {
                        original_opcode: opcode_name.to_string(),
                        new_opcode: new_opcode.to_string(),
                        mutated_expr: mutated,
                    },
                    path: context.path.clone(),
                    original: original.clone(),
                    source_line: source_line.clone(),
                    line_number,
                    column_number,
                }
            })
            .collect();

        Ok(mutants)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(yul_expr) = ctxt.yul_expr {
            if let yul::ExprKind::Call(call) = &yul_expr.kind {
                return self.opcode_mutations.contains_key(call.name.as_str());
            }
        }
        false
    }
}

/// Helper to generate a mutated Yul expression string.
/// Given "add(x, y)" and mutation "sub", returns "sub(x, y)".
pub fn mutate_yul_call(original: &str, original_opcode: &str, new_opcode: &str) -> String {
    original.replacen(original_opcode, new_opcode, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_mutations_exist() {
        let mutator = AssemblyMutator::new();

        // Arithmetic
        assert!(mutator.get_mutations("add").unwrap().contains(&"sub"));
        assert!(mutator.get_mutations("mul").unwrap().contains(&"div"));

        // Comparison
        assert!(mutator.get_mutations("lt").unwrap().contains(&"gt"));
        assert!(mutator.get_mutations("slt").unwrap().contains(&"sgt"));

        // Bitwise
        assert!(mutator.get_mutations("and").unwrap().contains(&"or"));
        assert!(mutator.get_mutations("shl").unwrap().contains(&"shr"));
    }

    #[test]
    fn test_mutate_yul_call() {
        assert_eq!(mutate_yul_call("add(x, y)", "add", "sub"), "sub(x, y)");
        assert_eq!(mutate_yul_call("lt(a, b)", "lt", "gt"), "gt(a, b)");
        assert_eq!(mutate_yul_call("shl(8, value)", "shl", "shr"), "shr(8, value)");
    }

    #[test]
    fn test_nested_call_mutation() {
        // Only first occurrence should be replaced
        let result = mutate_yul_call("add(add(x, y), z)", "add", "sub");
        assert_eq!(result, "sub(add(x, y), z)");
    }
}
