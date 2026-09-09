use super::IncorrectERC20Interface;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_elementary},
};
use solar::sema::{Gcx, hir};

declare_forge_lint!(
    INCORRECT_ERC20_INTERFACE,
    Severity::Med,
    "incorrect-erc20-interface",
    "incorrect ERC20 function interface"
);

/// ERC20 functions as `(name, parameter types, return types)`.
const ERC20_FUNCTIONS: &[(&str, &[&str], &[&str])] = &[
    ("transfer", &["address", "uint256"], &["bool"]),
    ("transferFrom", &["address", "address", "uint256"], &["bool"]),
    ("approve", &["address", "uint256"], &["bool"]),
    ("allowance", &["address", "address"], &["uint256"]),
    ("balanceOf", &["address"], &["uint256"]),
    ("totalSupply", &[], &["uint256"]),
];

impl<'gcx> LateLintPass<'gcx> for IncorrectERC20Interface {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract: &'gcx hir::Contract<'gcx>,
    ) {
        let inherits = |names: &[&str]| {
            contract
                .linearized_bases
                .iter()
                .any(|base| names.contains(&gcx.hir.contract(*base).name.as_str()))
        };
        // ERC721 tokens offer functions similar to ERC20 that are not compatible with it.
        if !inherits(&["ERC20", "IERC20"]) || inherits(&["ERC721", "IERC721"]) {
            return;
        }
        let matches = |vars: &[hir::VariableId], expected: &[&str]| {
            vars.len() == expected.len()
                && vars.iter().zip(expected).all(|(&id, &ty)| is_elementary(&gcx.hir, id, ty))
        };
        let functions = contract.items.iter().filter_map(|id| id.as_function());
        for func in functions.map(|id| gcx.hir.function(id)) {
            let Some(name) = func.name.filter(|_| func.kind.is_function()) else { continue };
            if ERC20_FUNCTIONS.iter().any(|(n, params, returns)| {
                *n == name.as_str()
                    && matches(func.parameters, params)
                    && !matches(func.returns, returns)
            }) {
                ctx.emit(&INCORRECT_ERC20_INTERFACE, func.span);
            }
        }
    }
}
