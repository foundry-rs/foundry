use crate::{
    executor::{backend::LocalForkId, inspector::Cheatcodes},
    utils::b160_to_h160,
};
use alloy_primitives::Address;
use foundry_common::fmt::UIfmt;
use foundry_utils::types::ToEthers;
use itertools::Itertools;

/// Represents possible diagnostic cases on revert
#[derive(Debug, Clone)]
pub enum RevertDiagnostic {
    /// The `contract` does not exist on the `active` fork but exist on other fork(s)
    ContractExistsOnOtherForks {
        contract: Address,
        active: LocalForkId,
        available_on: Vec<LocalForkId>,
    },
    ContractDoesNotExist {
        contract: Address,
        active: LocalForkId,
        persistent: bool,
    },
}

// === impl RevertDiagnostic ===

impl RevertDiagnostic {
    /// Converts the diagnostic to a readable error message
    pub fn to_error_msg(&self, cheats: &Cheatcodes) -> String {
        let get_label = |addr| cheats.labels.get(addr).cloned().unwrap_or_else(|| addr.pretty());

        match self {
            RevertDiagnostic::ContractExistsOnOtherForks { contract, active, available_on } => {
                let contract_label = get_label(&(*contract).to_ethers());

                format!(
                    r#"Contract {} does not exist on active fork with id `{}`
        But exists on non active forks: `[{}]`"#,
                    contract_label,
                    active,
                    available_on.iter().format(", ")
                )
            }
            RevertDiagnostic::ContractDoesNotExist { contract, persistent, .. } => {
                let contract_label = get_label(&b160_to_h160(*contract));
                if *persistent {
                    format!("Contract {contract_label} does not exist")
                } else {
                    format!("Contract {contract_label} does not exist and is not marked as persistent, see `vm.makePersistent()`")
                }
            }
        }
    }
}
