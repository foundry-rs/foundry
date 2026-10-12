//! Native script sender overrides and transaction collection.

use super::{ApplyError, Cheatcodes};
use crate::{BroadcastableTransaction, Error, script::Broadcast};
use alloy_primitives::{Address, U256};
use alloy_rpc_types::{TransactionInput, TransactionRequest};
use evm2::interpreter::{Interpreter, Message, MessageKind, derive_create_destination};
use foundry_common::{FoundryTransactionBuilder, TransactionMaybeSigned};
use foundry_evm_core::ethereum::FoundryEvmTypes;

impl Cheatcodes {
    pub(super) fn start_broadcast(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        signer: Option<Address>,
        single_call: bool,
    ) -> Result<(), ApplyError> {
        let depth = usize::from(message.depth.saturating_sub(1));
        if self.pranks.range(..=depth).next_back().is_some() {
            return Err(Error::from(
                "you have an active prank; broadcasting and pranks are not compatible",
            )
            .into());
        }
        if self.broadcast.is_some() {
            return Err(Error::from("a broadcast is active already").into());
        }
        let host = interp.host();
        let original_origin =
            host.ext().origin_override.or(host.ext().transaction_origin).unwrap_or(message.caller);
        let signer = match (signer, &self.wallets) {
            (Some(signer), _) => signer,
            (None, Some(wallets)) => {
                let mut wallets = wallets.inner.lock();
                if let Some(sender) = wallets.provided_sender {
                    sender
                } else {
                    let signers = wallets
                        .multi_wallet
                        .signers()
                        .map_err(|error| Error::from(error.to_string()))?;
                    if signers.len() == 1 {
                        *signers.keys().next().expect("one signer")
                    } else {
                        original_origin
                    }
                }
            }
            (None, None) => original_origin,
        };
        host.state_mut().account(&signer)?;
        self.broadcast = Some(Broadcast {
            new_origin: signer,
            original_caller: message.caller,
            original_origin,
            depth,
            single_call,
            #[cfg(feature = "revm")]
            deploy_from_code: false,
        });
        Ok(())
    }

    pub(super) fn broadcast_message(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Result<(), ApplyError> {
        let Some(broadcast) = &self.broadcast else { return Ok(()) };
        let fixed_gas_limit = message.gas_limit >= 21_000 && !self.dynamic_gas_limit;
        self.dynamic_gas_limit = false;
        let depth = usize::from(message.depth.saturating_sub(1));
        let synthetic =
            self.deploy_code_depth.is_some_and(|depth| depth.checked_add(1) == Some(message.depth));
        if depth != broadcast.depth && !synthetic || message.caller != broadcast.original_caller {
            return Ok(());
        }
        let broadcast = broadcast.clone();
        let host = interp.host();
        message.caller = broadcast.new_origin;
        host.ext_mut().origin_override = Some(broadcast.new_origin);
        if message.kind == MessageKind::StaticCall || message.caller_is_static {
            if broadcast.single_call {
                return Err(Error::from(
                    "`staticcall`s are not allowed after `broadcast`; use `startBroadcast` instead",
                )
                .into());
            }
            return Ok(());
        }
        let nonce =
            host.state_mut().account_info_untracked(&message.caller)?.map_or(0, |info| info.nonce);
        let mut request = TransactionRequest {
            from: Some(message.caller),
            value: Some(message.value),
            input: TransactionInput::new(message.input.clone()),
            nonce: Some(nonce),
            chain_id: Some(host.ext().chain_id_override.unwrap_or(host.version().chain_id)),
            ..Default::default()
        };
        if message.kind.is_create() {
            message.destination = derive_create_destination(
                message.kind,
                &message.caller,
                &message.salt,
                &message.input,
                nonce,
            );
            message.call_target = message.destination;
            message.code_address = message.destination;
            if message.kind == MessageKind::Create2 {
                request.to = Some(self.config.evm_opts.create2_deployer.into());
                request.input = TransactionInput::new(
                    [message.salt.as_slice(), message.input.as_ref()].concat().into(),
                );
                if !self.should_isolate(message) {
                    host.state_mut().account(&message.caller)?.set_nonce(nonce.saturating_add(1));
                }
            }
        } else {
            request.to = Some(message.destination.into());
            if fixed_gas_limit {
                request.gas = Some(message.gas_limit);
            }
            let authorizations = std::mem::take(&mut self.active_delegations);
            if let Some(sidecar) = self.active_blob_sidecar.take() {
                if !authorizations.is_empty() {
                    return Err(Error::from("both delegation and blob are active; `attachBlob` and `attachDelegation` are not compatible").into());
                }
                request.set_blob_sidecar(sidecar);
            }
            let chain_id = request.chain_id.expect("native chain id");
            // Attaching already updates local code. Consume authorization nonces before the
            // sender increment, including the sender's extra transaction nonce.
            for auth in &authorizations {
                if (!auth.chain_id.is_zero() && auth.chain_id != U256::from(chain_id))
                    || auth.nonce() == u64::MAX
                {
                    continue;
                }
                if let Ok(authority) = auth.recover_authority() {
                    let mut account = host.state_mut().account(&authority)?;
                    let current = account.get().map_or(0, |info| info.nonce);
                    if auth.nonce()
                        == current.saturating_add(u64::from(authority == message.caller))
                    {
                        account.set_nonce(current.saturating_add(1));
                    }
                }
            }
            if !authorizations.is_empty() {
                request.authorization_list = Some(authorizations);
            }
            if !self.should_isolate(message) {
                host.state_mut().account(&message.caller)?.bump_nonce();
            }
        }
        self.transactions.push_back(BroadcastableTransaction {
            rpc: self.forks.active_url().or_else(|| self.config.evm_opts.fork_url.clone()),
            transaction: TransactionMaybeSigned::new(request),
            contract_address: None,
        });
        Ok(())
    }

    pub(super) fn finish_broadcast(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
    ) {
        if let Some(broadcast) = &self.broadcast
            && usize::from(message.depth.saturating_sub(1)) == broadcast.depth
        {
            interp.host().ext_mut().origin_override = Some(broadcast.original_origin);
            if broadcast.single_call {
                self.broadcast = None;
            }
        }
    }
}
