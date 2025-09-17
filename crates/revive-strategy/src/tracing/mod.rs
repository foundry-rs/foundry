use alloy_primitives::{Address, Bytes, U256 as RU256};
use foundry_cheatcodes::Ecx;
use polkadot_sdk::{
    frame_support::weights::Weight,
    pallet_revive::{
        Config, Pallet, U256,
        evm::{
            CallTrace, CallTracer, PrestateTrace, PrestateTraceInfo, PrestateTracer,
            PrestateTracerConfig,
        },
        tracing::trace as trace_revive,
    },
};
use revm::{context::JournalTr, state::Bytecode};

// Traces the execution inside pallet_revive.
// This is a temporary solution to the fact that custom Tracer is not implementable for the time
// being.
pub fn trace<T: Config, R, F: FnOnce() -> R>(f: F) -> (R, Option<CallTrace<U256>>, PrestateTrace) {
    let mut call_tracer = CallTracer::new(
        Default::default(),
        Pallet::<revive_env::Runtime>::evm_gas_from_weight as fn(Weight) -> U256,
    );

    let mut prestate_tracer: PrestateTracer<revive_env::Runtime> =
        PrestateTracer::new(PrestateTracerConfig {
            diff_mode: true,
            disable_storage: false,
            disable_code: false,
        });

    let result = trace_revive(&mut prestate_tracer, || trace_revive(&mut call_tracer, f));
    let prestate_trace = prestate_tracer.collect_trace();
    let calls = call_tracer.collect_trace();
    (result, calls, prestate_trace)
}

/// Applies `PrestateTrace` diffs to the revm state
pub fn apply_prestate_trace(prestate_trace: PrestateTrace, ecx: Ecx<'_, '_, '_>) {
    match prestate_trace {
        polkadot_sdk::pallet_revive::evm::PrestateTrace::DiffMode { pre: _, post } => {
            for (key, PrestateTraceInfo { balance, nonce, code, storage }) in post {
                let address = Address::from_slice(key.as_bytes());
                let account = ecx
                    .journaled_state
                    .load_account(address)
                    .expect("account could not be loaded")
                    .data;

                account.mark_touch();

                if let Some(balance) = balance {
                    account.info.balance = RU256::from_limbs(balance.0);
                };

                if let Some(nonce) = nonce {
                    account.info.nonce = nonce.into();
                };

                if let Some(code) = code {
                    let account =
                        ecx.journaled_state.state.get_mut(&address).expect("account is loaded");
                    let bytecode = Bytecode::new_raw(Bytes::from(code.0));
                    account.info.code_hash = bytecode.hash_slow();
                    account.info.code = Some(bytecode);
                }
                ecx.journaled_state.load_account(address).expect("account could not be loaded");

                ecx.journaled_state.touch(address);
                for (slot, entry) in storage {
                    let key = RU256::from_be_slice(&slot.0);
                    if let Some(e_entry) = entry {
                        let entry = RU256::from_be_slice(&e_entry.0);

                        ecx.journaled_state.sstore(address, key, entry).expect("to succeed");
                    }
                }
            }
        }
        _ => panic!("Can't happen"),
    };
}
