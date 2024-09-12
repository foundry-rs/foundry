use crate::inspector::Cheatcodes;
use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::backend::DatabaseExt;
use revm::{
    interpreter::{CreateInputs, CreateScheme, EOFCreateInputs, EOFCreateKind},
    InnerEvmContext,
};

/// Common behaviour of legacy and EOF create inputs.
pub(crate) trait CommonCreateInput<DB: DatabaseExt> {
    fn caller(&self) -> Address;
    fn gas_limit(&self) -> u64;
    fn value(&self) -> U256;
    fn init_code(&self) -> Bytes;
    fn scheme(&self) -> Option<CreateScheme>;
    fn set_caller(&mut self, caller: Address);
    fn log_debug(&self, cheatcode: &mut Cheatcodes, scheme: &CreateScheme);
    fn allow_cheatcodes(
        &self,
        cheatcodes: &mut Cheatcodes,
        ecx: &mut InnerEvmContext<DB>,
    ) -> Address;
    fn computed_created_address(&self) -> Option<Address>;
}

impl<DB: DatabaseExt> CommonCreateInput<DB> for &mut CreateInputs {
    fn caller(&self) -> Address {
        self.caller
    }
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
    fn value(&self) -> U256 {
        self.value
    }
    fn init_code(&self) -> Bytes {
        self.init_code.clone()
    }
    fn scheme(&self) -> Option<CreateScheme> {
        Some(self.scheme)
    }
    fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }
    fn log_debug(&self, cheatcode: &mut Cheatcodes, scheme: &CreateScheme) {
        let kind = match scheme {
            CreateScheme::Create => "create",
            CreateScheme::Create2 { .. } => "create2",
        };
        debug!(target: "cheatcodes", tx=?cheatcode.broadcastable_transactions.back().unwrap(), "broadcastable {kind}");
    }
    fn allow_cheatcodes(
        &self,
        cheatcodes: &mut Cheatcodes,
        ecx: &mut InnerEvmContext<DB>,
    ) -> Address {
        let old_nonce = ecx
            .journaled_state
            .state
            .get(&self.caller)
            .map(|acc| acc.info.nonce)
            .unwrap_or_default();
        let created_address = self.created_address(old_nonce);
        cheatcodes.allow_cheatcodes_on_create(ecx, self.caller, created_address);
        created_address
    }
    fn computed_created_address(&self) -> Option<Address> {
        None
    }
}

impl<DB: DatabaseExt> CommonCreateInput<DB> for &mut EOFCreateInputs {
    fn caller(&self) -> Address {
        self.caller
    }
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
    fn value(&self) -> U256 {
        self.value
    }
    fn init_code(&self) -> Bytes {
        match &self.kind {
            EOFCreateKind::Tx { initdata } => initdata.clone(),
            EOFCreateKind::Opcode { initcode, .. } => initcode.raw.clone(),
        }
    }
    fn scheme(&self) -> Option<CreateScheme> {
        None
    }
    fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }
    fn log_debug(&self, cheatcode: &mut Cheatcodes, _scheme: &CreateScheme) {
        debug!(target: "cheatcodes", tx=?cheatcode.broadcastable_transactions.back().unwrap(), "broadcastable eofcreate");
    }
    fn allow_cheatcodes(
        &self,
        cheatcodes: &mut Cheatcodes,
        ecx: &mut InnerEvmContext<DB>,
    ) -> Address {
        let created_address =
            <&mut EOFCreateInputs as CommonCreateInput<DB>>::computed_created_address(self)
                .unwrap_or_default();
        cheatcodes.allow_cheatcodes_on_create(ecx, self.caller, created_address);
        created_address
    }
    fn computed_created_address(&self) -> Option<Address> {
        self.kind.created_address().copied()
    }
}
