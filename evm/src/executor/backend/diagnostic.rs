use crate::{
    executor::{backend::LocalForkId, inspector::Cheatcodes},
    Address,
};
use foundry_common::fmt::UIfmt;

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
    },
}

// === impl RevertDiagnostic ===

impl RevertDiagnostic {
    /// Converts the diagnostic to a readable error message
    pub fn to_error_msg(&self, cheats: &Cheatcodes) -> String {
        let get_label = |addr| cheats.labels.get(addr).cloned().unwrap_or_else(|| addr.pretty());

        match self {
            RevertDiagnostic::ContractExistsOnOtherForks { contract, active, available_on } => {
                let contract_label = get_label(contract);

                format!(
                    r#"Contract {} does not exists on active fork with id `{}`
        But exists on non active forks: `{:?}`"#,
                    contract_label, active, available_on
                )
            }
            RevertDiagnostic::ContractDoesNotExist { contract, .. } => {
                let contract_label = get_label(contract);
                format!("Contract {} does not exists", contract_label)
            }
        }
    }
}
