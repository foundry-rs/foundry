use crate::substrate_node::{
    host::{PublicKeyToHashOverride, SenderAddressRecoveryOverride},
    service::{Backend, backend::StorageOverrides},
};
use parking_lot::Mutex;
use polkadot_sdk::{
    parachains_common::{Hash, opaque::Block},
    sc_client_api::{Backend as _, CallExecutor, execution_extensions::ExecutionExtensions},
    sc_executor::{self, RuntimeVersion, RuntimeVersionOf},
    sc_service,
    sp_api::{CallContext, ProofRecorder},
    sp_blockchain::{self, HeaderBackend},
    sp_core, sp_externalities, sp_io,
    sp_runtime::{generic::BlockId, traits::HashingFor},
    sp_state_machine::{OverlayedChanges, StorageProof},
    sp_storage::ChildInfo,
    sp_version,
    sp_wasm_interface::ExtendedHostFunctions,
};
use std::{cell::RefCell, sync::Arc};

/// Wasm executor which overrides the signature checking host functions for impersonation.
pub type WasmExecutor = sc_executor::WasmExecutor<
    ExtendedHostFunctions<
        ExtendedHostFunctions<sp_io::SubstrateHostFunctions, SenderAddressRecoveryOverride>,
        PublicKeyToHashOverride,
    >,
>;

type InnerLocalCallExecutor = sc_service::client::LocalCallExecutor<Block, Backend, WasmExecutor>;

#[derive(Clone)]
/// State-injecting executor implementation.
pub struct Executor {
    inner: InnerLocalCallExecutor,
    storage_overrides: Arc<Mutex<StorageOverrides>>,
    backend: Arc<Backend>,
}

impl Executor {
    pub fn new(
        inner: InnerLocalCallExecutor,
        storage_overrides: Arc<Mutex<StorageOverrides>>,
        backend: Arc<Backend>,
    ) -> Self {
        Self { inner, storage_overrides, backend }
    }

    fn apply_overrides(&self, hash: &Hash, overlay: &mut OverlayedChanges<HashingFor<Block>>) {
        let overrides = { self.storage_overrides.lock().get(hash) };
        let Some(overrides) = overrides else { return };

        for (key, val) in overrides.top {
            overlay.set_storage(key, val);
        }

        for (child_key, child_map) in overrides.children {
            let child_info = ChildInfo::new_default_from_vec(child_key);

            for (key, val) in child_map {
                overlay.set_child_storage(&child_info, key, val);
            }
        }
    }
}

impl CallExecutor<Block> for Executor {
    type Error = <InnerLocalCallExecutor as CallExecutor<Block>>::Error;

    type Backend = Backend;

    fn execution_extensions(&self) -> &ExecutionExtensions<Block> {
        self.inner.execution_extensions()
    }

    fn call(
        &self,
        at_hash: Hash,
        method: &str,
        call_data: &[u8],
        context: CallContext,
    ) -> sp_blockchain::Result<Vec<u8>> {
        let at_number =
            self.backend.blockchain().expect_block_number_from_id(&BlockId::Hash(at_hash))?;
        let extensions = self.execution_extensions().extensions(at_hash, at_number);

        let mut changes = OverlayedChanges::default();

        self.apply_overrides(&at_hash, &mut changes);

        self.contextual_call(
            at_hash,
            method,
            call_data,
            &RefCell::new(changes),
            &None,
            context,
            &RefCell::new(extensions),
        )
    }

    fn contextual_call(
        &self,
        at_hash: Hash,
        method: &str,
        call_data: &[u8],
        changes: &RefCell<OverlayedChanges<HashingFor<Block>>>,
        recorder: &Option<ProofRecorder<Block>>,
        call_context: CallContext,
        extensions: &RefCell<sp_externalities::Extensions>,
    ) -> Result<Vec<u8>, sp_blockchain::Error> {
        let apply_overrides = (method == "Core_initialize_block"
            && call_context == CallContext::Onchain)
            || call_context == CallContext::Offchain;

        if apply_overrides {
            self.apply_overrides(&at_hash, &mut changes.borrow_mut());
        }

        self.inner.contextual_call(
            at_hash,
            method,
            call_data,
            changes,
            recorder,
            call_context,
            extensions,
        )
    }

    fn runtime_version(&self, at_hash: Hash) -> sp_blockchain::Result<RuntimeVersion> {
        CallExecutor::runtime_version(&self.inner, at_hash)
    }

    fn prove_execution(
        &self,
        at_hash: Hash,
        method: &str,
        call_data: &[u8],
    ) -> sp_blockchain::Result<(Vec<u8>, StorageProof)> {
        self.inner.prove_execution(at_hash, method, call_data)
    }
}

impl RuntimeVersionOf for Executor {
    fn runtime_version(
        &self,
        ext: &mut dyn sp_externalities::Externalities,
        runtime_code: &sp_core::traits::RuntimeCode<'_>,
    ) -> Result<sp_version::RuntimeVersion, sc_executor::error::Error> {
        RuntimeVersionOf::runtime_version(&self.inner, ext, runtime_code)
    }
}
