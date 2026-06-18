use super::IncorrectERC721Interface;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::hir;

declare_forge_lint!(
    INCORRECT_ERC721_INTERFACE,
    Severity::Med,
    "incorrect-erc721-interface",
    "incorrect ERC721 function interface"
);

impl<'hir> LateLintPass<'hir> for IncorrectERC721Interface {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        // Check if the contract is a possible ERC721 by name or inheritance.
        let is_erc721 = contract.linearized_bases.iter().any(|base_id| {
            let name = hir.contract(*base_id).name.as_str();
            name == "ERC721" || name == "IERC721"
        });

        if !is_erc721 {
            return;
        }

        // Check each function in the contract for incorrect ERC721 signatures.
        for item_id in contract.items {
            let Some(fid) = item_id.as_function() else { continue };
            let func = hir.function(fid);

            if !func.kind.is_function() {
                continue;
            }

            let Some(name) = func.name else { continue };

            if has_incorrect_erc721_signature(hir, name.as_str(), func.parameters, func.returns) {
                ctx.emit(&INCORRECT_ERC721_INTERFACE, func.span);
            }
        }
    }
}

/// Checks if a function signature does not match the expected ERC721 (or ERC165) specification.
///
/// Returns `true` if the function name and parameter types match an ERC721 function but the return
/// types are incorrect.
fn has_incorrect_erc721_signature(
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
        // function balanceOf(address) external view returns (uint256)
        "balanceOf" if params_match(parameters, &["address"]) => {
            !returns_match(returns, &["uint256"])
        }
        // function ownerOf(uint256) external view returns (address)
        "ownerOf" if params_match(parameters, &["uint256"]) => {
            !returns_match(returns, &["address"])
        }
        // function safeTransferFrom(address,address,uint256,bytes) external
        "safeTransferFrom"
            if params_match(parameters, &["address", "address", "uint256", "bytes"]) =>
        {
            !returns_match(returns, &[])
        }
        // function safeTransferFrom(address,address,uint256) external
        "safeTransferFrom" if params_match(parameters, &["address", "address", "uint256"]) => {
            !returns_match(returns, &[])
        }
        // function transferFrom(address,address,uint256) external
        "transferFrom" if params_match(parameters, &["address", "address", "uint256"]) => {
            !returns_match(returns, &[])
        }
        // function approve(address,uint256) external
        "approve" if params_match(parameters, &["address", "uint256"]) => {
            !returns_match(returns, &[])
        }
        // function setApprovalForAll(address,bool) external
        "setApprovalForAll" if params_match(parameters, &["address", "bool"]) => {
            !returns_match(returns, &[])
        }
        // function getApproved(uint256) external view returns (address)
        "getApproved" if params_match(parameters, &["uint256"]) => {
            !returns_match(returns, &["address"])
        }
        // function isApprovedForAll(address,address) external view returns (bool)
        "isApprovedForAll" if params_match(parameters, &["address", "address"]) => {
            !returns_match(returns, &["bool"])
        }
        // ERC165: function supportsInterface(bytes4) external view returns (bool)
        "supportsInterface" if params_match(parameters, &["bytes4"]) => {
            !returns_match(returns, &["bool"])
        }
        _ => false,
    }
}
