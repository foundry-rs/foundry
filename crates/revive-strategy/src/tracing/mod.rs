use polkadot_sdk::{
    frame_support::{traits::IsType, weights::Weight},
    pallet_revive::{
        evm::{CallTrace, CallTracer, PrestateTrace, PrestateTracer, PrestateTracerConfig},
        tracing::trace as trace_revive,
        BalanceOf, Config, MomentOf, Pallet, H256, U256,
    },
    sp_runtime::traits::Bounded,
};

// Traces the execution inside pallet_revive.
// This is a temporary solution to the fact that custom Tracer is not implementable for the time
// being.
pub fn trace<T: Config, R, F: FnOnce() -> R>(f: F) -> (R, Option<CallTrace<U256>>, PrestateTrace)
where
    BalanceOf<T>: Into<U256> + TryFrom<U256> + Bounded,
    MomentOf<T>: Into<U256>,
    T::Hash: IsType<H256>,
    T::Nonce: Into<u32>,
{
    let mut call_tracer =
        CallTracer::new(Default::default(), Pallet::<T>::evm_gas_from_weight as fn(Weight) -> U256);

    let mut prestate_tracer: PrestateTracer<T> = PrestateTracer::new(PrestateTracerConfig {
        diff_mode: true,
        disable_storage: false,
        disable_code: false,
    });

    let result = trace_revive(&mut call_tracer, || trace_revive(&mut prestate_tracer, f));
    let prestate_trace = prestate_tracer.collect_trace();
    (result, call_tracer.collect_trace(), prestate_trace)
}
