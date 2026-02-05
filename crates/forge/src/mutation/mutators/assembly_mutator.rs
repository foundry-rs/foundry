use std::collections::HashMap;

use eyre::Result;
use solar::ast::yul;

use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

pub struct AssemblyMutator {
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

        opcode_mutations.insert("lt", vec!["gt", "eq", "slt"]);
        opcode_mutations.insert("gt", vec!["lt", "eq", "sgt"]);
        opcode_mutations.insert("slt", vec!["sgt", "lt"]);
        opcode_mutations.insert("sgt", vec!["slt", "gt"]);
        opcode_mutations.insert("eq", vec!["lt", "gt"]);
        opcode_mutations.insert("iszero", vec!["not"]);

        opcode_mutations.insert("and", vec!["or", "xor"]);
        opcode_mutations.insert("or", vec!["and", "xor"]);
        opcode_mutations.insert("xor", vec!["and", "or"]);
        opcode_mutations.insert("not", vec!["iszero"]);
        opcode_mutations.insert("shl", vec!["shr", "sar"]);
        opcode_mutations.insert("shr", vec!["shl", "sar"]);
        opcode_mutations.insert("sar", vec!["shr", "shl"]);

        Self { opcode_mutations }
    }

    pub fn get_mutations(&self, opcode: &str) -> Option<&Vec<&'static str>> {
        self.opcode_mutations.get(opcode)
    }
}

impl Mutator for AssemblyMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let yul_expr = context.yul_expr.ok_or_else(|| eyre::eyre!("No Yul expression"))?;

        let call = match &yul_expr.kind {
            yul::ExprKind::Call(call) => call,
            _ => return Ok(vec![]),
        };

        let opcode_name = call.name.as_str();

        let alternatives = match self.get_mutations(opcode_name) {
            Some(alts) => alts,
            None => return Ok(vec![]),
        };

        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();

        let name_span = call.name.span;

        let mutants = alternatives
            .iter()
            .map(|&new_opcode| {
                let mutated = replace_at_span(&original, context.span, name_span, new_opcode);
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
                }
            })
            .collect();

        Ok(mutants)
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(yul_expr) = ctxt.yul_expr
            && let yul::ExprKind::Call(call) = &yul_expr.kind
        {
            return self.opcode_mutations.contains_key(call.name.as_str());
        }
        false
    }
}

fn replace_at_span(
    original: &str,
    outer_span: solar::ast::Span,
    target_span: solar::ast::Span,
    replacement: &str,
) -> String {
    let outer_lo = outer_span.lo().0 as usize;
    let target_lo = target_span.lo().0 as usize;
    let target_hi = target_span.hi().0 as usize;

    let rel_lo = target_lo - outer_lo;
    let rel_hi = target_hi - outer_lo;

    debug_assert!(
        rel_hi <= original.len(),
        "target span exceeds original text: rel_hi={rel_hi}, len={}",
        original.len()
    );
    let rel_hi = rel_hi.min(original.len());
    format!("{}{}{}", &original[..rel_lo], replacement, &original[rel_hi..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_mutations_exist() {
        let mutator = AssemblyMutator::new();

        assert!(mutator.get_mutations("add").unwrap().contains(&"sub"));
        assert!(mutator.get_mutations("mul").unwrap().contains(&"div"));

        assert!(mutator.get_mutations("lt").unwrap().contains(&"gt"));
        assert!(mutator.get_mutations("slt").unwrap().contains(&"sgt"));

        assert!(mutator.get_mutations("and").unwrap().contains(&"or"));
        assert!(mutator.get_mutations("shl").unwrap().contains(&"shr"));
    }

    #[test]
    fn test_no_mload_sload_mapping() {
        let mutator = AssemblyMutator::new();
        assert!(mutator.get_mutations("mload").is_none());
        assert!(mutator.get_mutations("sload").is_none());
    }
}
