//! Cheatcode ABIs for trace decoding.
//!
//! `Vm` is declared without `#[sol(abi)]`, which would generate a `Function` literal for each
//! cheatcode at compile time. The functions are built here from the cheatcode spec instead.

use alloy_json_abi::{Function, Param};
use foundry_cheatcodes_spec::{Cheatcodes, Enum, Struct};

/// Returns the ABI of every cheatcode, in spec order.
pub(super) fn functions() -> Vec<Function> {
    let Cheatcodes { structs, enums, cheatcodes, .. } = Cheatcodes::new();
    cheatcodes
        .iter()
        .map(|cheatcode| {
            let declaration = cheatcode.func.declaration.trim_end_matches(';');
            let mut func = Function::parse(declaration)
                .unwrap_or_else(|e| panic!("invalid cheatcode `{declaration}`: {e}"));
            for param in func.inputs.iter_mut().chain(&mut func.outputs) {
                resolve(param, &structs, &enums);
            }
            func
        })
        .collect()
}

/// Replaces spec struct and enum types in `param` with their ABI types: structs become tuples
/// and enums become `uint8`.
fn resolve(param: &mut Param, structs: &[Struct<'_>], enums: &[Enum<'_>]) {
    let (root, suffix) = param.ty.split_at(param.ty.find('[').unwrap_or(param.ty.len()));
    if let Some(strukt) = structs.iter().find(|s| s.name == root) {
        param.components = strukt
            .fields
            .iter()
            .map(|field| {
                let mut component = Param::parse(&format!("{} {}", field.ty, field.name))
                    .unwrap_or_else(|e| {
                        panic!("invalid field `{}.{}`: {e}", strukt.name, field.name)
                    });
                resolve(&mut component, structs, enums);
                component
            })
            .collect();
        param.ty = format!("tuple{suffix}");
    } else if enums.iter().any(|e| e.name == root) {
        param.ty = format!("uint8{suffix}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_match_spec() {
        let cheatcodes = Cheatcodes::new().cheatcodes;
        let functions = functions();
        assert_eq!(functions.len(), cheatcodes.len());
        for (func, cheatcode) in functions.iter().zip(cheatcodes.iter()) {
            assert_eq!(func.selector().0, cheatcode.func.selector_bytes, "{}", func.signature());
        }
    }
}
