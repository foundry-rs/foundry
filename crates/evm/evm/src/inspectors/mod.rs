//! EVM inspectors.

pub use foundry_cheatcodes::{self as cheatcodes, Cheatcodes, CheatsConfig};
use foundry_evm_core::InspectorExt;
pub use foundry_evm_coverage::CoverageCollector;
pub use foundry_evm_fuzz::Fuzzer;
pub use foundry_evm_traces::{StackSnapshotType, TracingInspector, TracingInspectorConfig};

use revm::{db::WrapDatabaseRef, Database, DatabaseRef};
pub use revm_inspectors::access_list::AccessListInspector;

mod chisel_state;
pub use chisel_state::ChiselState;

mod debugger;
pub use debugger::Debugger;

mod logs;
pub use logs::LogCollector;

mod stack;
pub use stack::{InspectorData, InspectorStack, InspectorStackBuilder};

use alloy_eips::eip2930::AccessList;
use alloy_primitives::Address;
/// To be used in Anvil
pub struct AnvilAccessListInspector {
    pub inner: AccessListInspector,
}

impl AnvilAccessListInspector {
    pub fn new(
        access_list: AccessList,
        from: Address,
        to: Address,
        precompiles: impl IntoIterator<Item = Address>,
    ) -> Self {
        Self { inner: AccessListInspector::new(access_list, from, to, precompiles) }
    }

    pub fn access_list(&self) -> AccessList {
        self.inner.access_list()
    }
}

impl<DB: Database> revm::Inspector<DB> for AnvilAccessListInspector {}

impl<DB: DatabaseRef> InspectorExt<WrapDatabaseRef<DB>> for AnvilAccessListInspector {}
