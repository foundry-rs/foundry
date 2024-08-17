use crate::backend::LocalForkId;
use alloy_primitives::Address;
use itertools::Itertools;
use std::collections::HashMap;

/// Represents possible diagnostic cases on revert
#[derive(Clone, Debug)]
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

impl RevertDiagnostic {
    /// Converts the diagnostic to a readable error message
    pub fn to_error_msg(&self, labels: &HashMap<Address, String>) -> String {
        let get_label =
            |addr: &Address| labels.get(addr).cloned().unwrap_or_else(|| addr.to_string());

        match self {
            Self::ContractExistsOnOtherForks { contract, active, available_on } => {
                let contract_label = get_label(contract);

                format!(
                    r#"Contract {} does not exist on active fork with id `{}`
        But exists on non active forks: `[{}]`"#,
                    contract_label,
                    active,
                    available_on.iter().format(", ")
                )
            }
            Self::ContractDoesNotExist { contract, persistent, .. } => {
                let contract_label = get_label(contract);
                if *persistent {
                    format!("Contract {contract_label} does not exist")
                } else {
                    format!("Contract {contract_label} does not exist and is not marked as persistent, see `vm.makePersistent()`")
                }
            }
        }
    }
}
