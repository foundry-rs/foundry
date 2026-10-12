//! Artifact deployment inside the owning native transaction.

use super::{ApplyError, Cheatcodes, DEFAULT_CREATE2_DEPLOYER_CODEHASH};
use crate::Error;
use alloy_primitives::{B256, Bytes, U256};
use alloy_sol_types::SolValue;
use evm2::{
    Evm, ExecutionConfig, Precompiles,
    bytecode::Bytecode,
    ethereum::ethereum_tx_registry,
    evm::{EmptyDB, State},
    interpreter::{
        Host, InstrStop, Interpreter, Message, MessageKind, MessageResult,
        derive_create_destination,
    },
};
use foundry_evm_core::ethereum::FoundryEvmTypes;
use std::mem;

impl Cheatcodes {
    pub(super) fn deploy_code(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        call: &Message<FoundryEvmTypes>,
        path: &str,
        args: Option<&Bytes>,
        value: U256,
        salt: Option<B256>,
    ) -> Result<Bytes, ApplyError> {
        if call.caller_is_static || call.kind == MessageKind::StaticCall {
            return Err(Error::from(Bytes::new()).into());
        }
        let mut input = crate::fs::get_artifact_code(&self.config, path, false)?.to_vec();
        if let Some(args) = args {
            input.extend_from_slice(args);
        }
        let input = Bytes::from(input);
        let host = interp.host();
        let origin = host.ext().origin_override;
        let depth = usize::from(call.depth.saturating_sub(1));
        let prank = self.pranks.get(&depth).copied();
        let mut caller = call.caller;
        if let Some(prank) = prank
            && let Some(changes) = prank.changes_for(depth, caller)
        {
            if let Some(new_caller) = changes.caller {
                caller = new_caller;
            }
            if let Some(new_origin) = changes.origin {
                host.ext_mut().origin_override = Some(new_origin);
            }
            if let Some(used) = changes.used {
                self.pranks.insert(prank.depth, used);
            }
        }
        let kind = if salt.is_some() { MessageKind::Create2 } else { MessageKind::Create };
        let salt = salt.unwrap_or_default();
        let nonce = host.state_mut().account_info_untracked(&caller)?.map_or(0, |info| info.nonce);
        let destination = derive_create_destination(kind, &caller, &salt, &input, nonce);
        let mut message = Message::<FoundryEvmTypes> {
            kind,
            depth: call.depth + 1,
            gas_limit: call.gas_limit,
            reservoir: call.reservoir,
            destination,
            call_target: destination,
            caller,
            code: Bytecode::new_legacy(input.clone()),
            code_address: destination,
            input,
            value,
            salt,
            ..Default::default()
        };
        let previous_depth = self.deploy_code_depth.replace(call.depth);
        let result = self.execute_owned_message(interp, &mut message);
        self.deploy_code_depth = previous_depth;
        let host = interp.host();
        host.ext_mut().origin_override = origin;
        if let Some(prank) = prank
            && prank.single_call
        {
            self.pranks.remove(&depth);
        }
        let result = result.map_err(ApplyError::Execution)?;
        if !result.stop.is_success() {
            return Err(Error::from(result.output).into());
        }
        let address =
            result.created_address.ok_or_else(|| Error::from("contract creation failed"))?;
        Ok(address.abi_encode().into())
    }

    pub(super) fn create2_factory(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
    ) -> Result<MessageResult<FoundryEvmTypes>, ApplyError> {
        let deployer = self.config.evm_opts.create2_deployer;
        let code = {
            let mut account = interp.host().state_mut().account(&deployer)?;
            let hash = account.code_hash();
            if hash == alloy_primitives::KECCAK256_EMPTY {
                return Ok(Self::result(
                    message,
                    InstrStop::Revert,
                    format!("missing CREATE2 deployer: {deployer}").into_bytes().into(),
                ));
            }
            if hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
                return Ok(Self::result(
                    message,
                    InstrStop::Revert,
                    "invalid CREATE2 deployer bytecode".into(),
                ));
            }
            account.load_code()?
        };
        let mut call = Message::<FoundryEvmTypes> {
            kind: MessageKind::Call,
            depth: message.depth,
            gas_limit: message.gas_limit,
            reservoir: message.reservoir,
            destination: deployer,
            call_target: deployer,
            caller: message.caller,
            input: [message.salt.as_slice(), message.input.as_ref()].concat().into(),
            value: message.value,
            code,
            code_address: deployer,
            ..Default::default()
        };
        let mut result =
            self.execute_owned_message(interp, &mut call).map_err(ApplyError::Execution)?;
        if result.stop.is_success() {
            match alloy_primitives::Address::try_from(result.output.as_ref()) {
                Ok(address) => result.created_address = Some(address),
                Err(_) => {
                    return Ok(Self::result(
                        message,
                        InstrStop::Revert,
                        "invalid CREATE2 factory output".into(),
                    ));
                }
            }
        }
        Ok(result)
    }

    /// Runs an owned inspector against the suspended parent's live transaction state.
    pub(super) fn execute_owned_message(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Result<MessageResult<FoundryEvmTypes>, evm2::ExecutionError> {
        let tx_env = interp.tx_env().clone();
        let host = interp.host();
        let spec = host.spec_id();
        let mut child = Evm::<FoundryEvmTypes>::new_with_execution_config_and_ext(
            ExecutionConfig::for_spec_and_version(spec, *host.version()),
            spec,
            *host.block(),
            ethereum_tx_registry(spec),
            EmptyDB::default(),
            Precompiles::base(spec),
            *host.ext(),
        );
        *child.state_mut() = mem::replace(host.state_mut(), State::new(EmptyDB::default()));
        child.set_inspector(mem::take(self));
        let result = Host::execute_message(&mut child, &tx_env, message);
        let inspector = child.clear_inspector().expect("native child inspector");
        // SAFETY: This child was given an owned `Self` immediately above. Inspector replacement
        // is prohibited during execution, and `Self` contains no borrowed host state. The
        // engine's checked extraction unnecessarily requires its database lifetime to be static.
        *self = *unsafe { Box::from_raw(Box::into_raw(inspector).cast::<Self>()) };
        *host.state_mut() = mem::replace(child.state_mut(), State::new(EmptyDB::default()));
        host.set_block(*child.block());
        *host.ext_mut() = *child.ext();
        result
    }
}
