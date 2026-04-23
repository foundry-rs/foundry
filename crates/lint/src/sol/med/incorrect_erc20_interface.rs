use super::IncorrectERC20Interface;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::hir;

declare_forge_lint!(
    INCORRECT_ERC20_INTERFACE,
    Severity::Med,
    "incorrect-erc20-interface",
    "incorrect ERC20 function interface"
);

impl<'hir> LateLintPass<'hir> for IncorrectERC20Interface {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        // Check if the contract is a possible ERC20 by name or inheritance.
        let is_erc20 = contract.linearized_bases.iter().any(|base_id| {
            let name = hir.contract(*base_id).name.as_str();
            name == "ERC20" || name == "IERC20"
        });

        if !is_erc20 {
            return;
        }

        // If this contract implements a function from ERC721, we can assume it is an ERC721 token.
        // These tokens offer functions which are similar to ERC20, but are not compatible.
        let is_erc721 = contract.linearized_bases.iter().any(|base_id| {
            let name = hir.contract(*base_id).name.as_str();
            name == "ERC721" || name == "IERC721"
        });

        if is_erc721 {
            return;
        }

        // Check each function in the contract for incorrect ERC20 signatures.
        for item_id in contract.items {
            let Some(fid) = item_id.as_function() else { continue };
            let func = hir.function(fid);

            if !func.kind.is_function() {
                continue;
            }

            let Some(name) = func.name else { continue };

            if has_incorrect_erc20_signature(hir, name.as_str(), func.parameters, func.returns) {
                ctx.emit(&INCORRECT_ERC20_INTERFACE, func.span);
            }
        }
    }
}

/// Checks if a function signature does not match the expected ERC20 specification.
///
/// Returns `true` if the function name and parameter types match an ERC20 function but the return
/// types are incorrect.
fn has_incorrect_erc20_signature(
    hir: &hir::Hir<'_>,
    name: &str,
    parameters: &[hir::VariableId],
    returns: &[hir::VariableId],
) -> bool {
    let is_type = |var_id: hir::VariableId, type_str: &str| {
        matches!(
            &hir.variable(var_id).ty.kind,
            hir::TypeKind::Elementary(ty) if ty.to_abi_str() == type_str
        )
    };

    let params_match = |params: &[hir::VariableId], expected: &[&str]| -> bool {
        params.len() == expected.len()
            && params.iter().zip(expected).all(|(&id, &ty)| is_type(id, ty))
    };

    let returns_match = |rets: &[hir::VariableId], expected: &[&str]| -> bool {
        rets.len() == expected.len() && rets.iter().zip(expected).all(|(&id, &ty)| is_type(id, ty))
    };

    match name {
        // function transfer(address,uint256) external returns (bool)
        "transfer" if params_match(parameters, &["address", "uint256"]) => {
            !returns_match(returns, &["bool"])
        }
        // function transferFrom(address,address,uint256) external returns (bool)
        "transferFrom" if params_match(parameters, &["address", "address", "uint256"]) => {
            !returns_match(returns, &["bool"])
        }
        // function approve(address,uint256) external returns (bool)
        "approve" if params_match(parameters, &["address", "uint256"]) => {
            !returns_match(returns, &["bool"])
        }
        // function allowance(address,address) external view returns (uint256)
        "allowance" if params_match(parameters, &["address", "address"]) => {
            !returns_match(returns, &["uint256"])
        }
        // function balanceOf(address) external view returns (uint256)
        "balanceOf" if params_match(parameters, &["address"]) => {
            !returns_match(returns, &["uint256"])
        }
        // function totalSupply() external view returns (uint256)
        "totalSupply" if params_match(parameters, &[]) => !returns_match(returns, &["uint256"]),
        _ => false,
    }
}
