use alloy_primitives::B256;
use alloy_provider::{Provider, network::AnyNetwork};
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_rpc_types::trace::geth::{
    CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingCallOptions,
    GethDebugTracingOptions, GethTrace,
};
use alloy_serde::WithOtherFields;
use eyre::Result;

/// Executes remote traces via debug_traceCall / debug_traceTransaction on a live RPC.
#[derive(Clone, Debug)]
pub struct RemoteRpcExecutor<P: Provider<AnyNetwork>> {
    provider: P,
}

impl<P> RemoteRpcExecutor<P>
where
    P: Provider<AnyNetwork> + Clone,
{
    /// Create a new remote RPC executor from an Alloy provider
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Perform a debug_traceCall using the built-in CallTracer
    pub async fn debug_trace_call(&self, tx: TransactionRequest) -> Result<GethTrace> {
        let opts = GethDebugTracingCallOptions::default().with_tracing_options(
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                .with_call_config(CallConfig::default().with_log()),
        );

        let trace = self
            .provider
            .debug_trace_call(WithOtherFields::new(tx), BlockId::latest(), opts)
            .await?;

        Ok(trace)
    }

    /// Perform a debug_traceTransaction using the built-in CallTracer
    pub async fn debug_trace_transaction(&self, hash: B256) -> Result<GethTrace> {
        let opts = GethDebugTracingOptions::default().with_tracer(GethDebugTracerType::from(
            GethDebugBuiltInTracerType::CallTracer,
        ));

        let trace = self.provider.debug_trace_transaction(hash, opts).await?;
        Ok(trace)
    }
}


