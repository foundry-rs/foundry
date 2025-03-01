//! Noop tracer response.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An empty frame response that's only an empty json object `{}`.
///
/// <https://github.com/ethereum/go-ethereum/blob/91cb6f863a965481e51d5d9c0e5ccd54796fd967/eth/tracers/native/noop.go#L34>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoopFrame(BTreeMap<(), ()>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geth::*;
    use similar_asserts::assert_eq;

    const DEFAULT: &str = r"{}";

    #[test]
    fn test_serialize_noop_trace() {
        let mut opts = GethDebugTracingCallOptions::default();
        opts.tracing_options.tracer =
            Some(GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::NoopTracer));

        assert_eq!(serde_json::to_string(&opts).unwrap(), r#"{"tracer":"noopTracer"}"#);
    }

    #[test]
    fn test_deserialize_noop_trace() {
        let _trace: NoopFrame = serde_json::from_str(DEFAULT).unwrap();
    }
}
