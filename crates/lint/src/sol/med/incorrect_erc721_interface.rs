use super::IncorrectERC721Interface;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_elementary},
};
use solar::sema::{Gcx, hir};

declare_forge_lint!(
    INCORRECT_ERC721_INTERFACE,
    Severity::Med,
    "incorrect-erc721-interface",
    "incorrect ERC721 function interface"
);

/// ERC721 (and ERC165) functions as `(name, parameter types, return types)`.
const ERC721_FUNCTIONS: &[(&str, &[&str], &[&str])] = &[
    ("balanceOf", &["address"], &["uint256"]),
    ("ownerOf", &["uint256"], &["address"]),
    ("safeTransferFrom", &["address", "address", "uint256", "bytes"], &[]),
    ("safeTransferFrom", &["address", "address", "uint256"], &[]),
    ("transferFrom", &["address", "address", "uint256"], &[]),
    ("approve", &["address", "uint256"], &[]),
    ("setApprovalForAll", &["address", "bool"], &[]),
    ("getApproved", &["uint256"], &["address"]),
    ("isApprovedForAll", &["address", "address"], &["bool"]),
    ("supportsInterface", &["bytes4"], &["bool"]),
];

impl<'gcx> LateLintPass<'gcx> for IncorrectERC721Interface {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract: &'gcx hir::Contract<'gcx>,
    ) {
        if !contract
            .linearized_bases
            .iter()
            .any(|base| matches!(gcx.hir.contract(*base).name.as_str(), "ERC721" | "IERC721"))
        {
            return;
        }
        let matches = |vars: &[hir::VariableId], expected: &[&str]| {
            vars.len() == expected.len()
                && vars.iter().zip(expected).all(|(&id, &ty)| is_elementary(&gcx.hir, id, ty))
        };
        let functions = contract.items.iter().filter_map(|id| id.as_function());
        for func in functions.map(|id| gcx.hir.function(id)) {
            let Some(name) = func.name.filter(|_| func.kind.is_function()) else { continue };
            if ERC721_FUNCTIONS.iter().any(|(n, params, returns)| {
                *n == name.as_str()
                    && matches(func.parameters, params)
                    && !matches(func.returns, returns)
            }) {
                ctx.emit(&INCORRECT_ERC721_INTERFACE, func.span);
            }
        }
    }
}
