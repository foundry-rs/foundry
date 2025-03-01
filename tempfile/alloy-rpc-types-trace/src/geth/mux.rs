//! Geth `muxTracer` types.

use crate::geth::{GethDebugBuiltInTracerType, GethDebugTracerConfig, GethTrace};
use alloy_primitives::map::HashMap;
use serde::{Deserialize, Serialize};

/// A `muxTracer` config that contains the configuration for running multiple tracers in one go.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxConfig(pub HashMap<GethDebugBuiltInTracerType, Option<GethDebugTracerConfig>>);

/// A `muxTracer` frame response that contains the results of multiple tracers
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuxFrame(pub HashMap<GethDebugBuiltInTracerType, GethTrace>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geth::*;
    use similar_asserts::assert_eq;

    const FOUR_BYTE_FRAME: &str = r#"{
        "0x27dc297e-128": 1,
        "0x38cc4831-0": 2,
        "0x524f3889-96": 1,
        "0xadf59f99-288": 1,
        "0xc281d19e-0": 1
    }"#;

    const CALL_FRAME_WITH_LOG: &str = include_str!("../../test_data/call_tracer/with_log.json");

    const PRESTATE_FRAME: &str = r#"{
      "0x0000000000000000000000000000000000000002": {
        "balance": "0x0"
      },
      "0x008b3b2f992c0e14edaa6e2c662bec549caa8df1": {
        "balance": "0x2638035a26d133809"
      },
      "0x35a9f94af726f07b5162df7e828cc9dc8439e7d0": {
        "balance": "0x7a48734599f7284",
        "nonce": 1133
      },
      "0xc8ba32cab1757528daf49033e3673fae77dcf05d": {
        "balance": "0x0",
        "code": "0x",
        "nonce": 1,
        "storage": {
          "0x0000000000000000000000000000000000000000000000000000000000000000": "0x000000000000000000000000000000000000000000000000000000000024aea6",
          "0x59fb7853eb21f604d010b94c123acbeae621f09ce15ee5d7616485b1e78a72e9": "0x00000000000000c42b56a52aedf18667c8ae258a0280a8912641c80c48cd9548",
          "0x8d8ebb65ec00cb973d4fe086a607728fd1b9de14aa48208381eed9592f0dee9a": "0x00000000000000784ae4881e40b1f5ebb4437905fbb8a5914454123b0293b35f",
          "0xff896b09014882056009dedb136458f017fcef9a4729467d0d00b4fd413fb1f1": "0x000000000000000e78ac39cb1c20e9edc753623b153705d0ccc487e31f9d6749"
        }
      }
    }"#;

    #[test]
    fn test_serialize_mux_tracer_config() {
        let mut opts = GethDebugTracingCallOptions::default();
        opts.tracing_options.tracer =
            Some(GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::MuxTracer));

        let call_config = CallConfig { only_top_call: Some(true), with_log: Some(true) };
        let prestate_config = PreStateConfig { diff_mode: Some(true), ..Default::default() };

        opts.tracing_options.tracer_config = MuxConfig(HashMap::from_iter([
            (GethDebugBuiltInTracerType::FourByteTracer, None),
            (GethDebugBuiltInTracerType::CallTracer, Some(call_config.into())),
            (GethDebugBuiltInTracerType::PreStateTracer, Some(prestate_config.into())),
        ]))
        .into();

        assert_eq!(
            serde_json::to_string(&opts).unwrap(),
            r#"{"tracer":"muxTracer","tracerConfig":{"4byteTracer":null,"callTracer":{"onlyTopCall":true,"withLog":true},"prestateTracer":{"diffMode":true}}}"#,
        );
    }

    #[test]
    fn test_deserialize_mux_frame() {
        let expected = HashMap::from([
            (
                GethDebugBuiltInTracerType::FourByteTracer,
                GethTrace::FourByteTracer(serde_json::from_str(FOUR_BYTE_FRAME).unwrap()),
            ),
            (
                GethDebugBuiltInTracerType::CallTracer,
                GethTrace::CallTracer(serde_json::from_str(CALL_FRAME_WITH_LOG).unwrap()),
            ),
            (
                GethDebugBuiltInTracerType::PreStateTracer,
                GethTrace::PreStateTracer(serde_json::from_str(PRESTATE_FRAME).unwrap()),
            ),
        ]);

        let raw_frame = serde_json::to_string(&expected).unwrap();
        let trace: MuxFrame = serde_json::from_str(&raw_frame).unwrap();

        assert_eq!(
            trace.0[&GethDebugBuiltInTracerType::FourByteTracer],
            expected[&GethDebugBuiltInTracerType::FourByteTracer]
        );
        assert_eq!(
            trace.0[&GethDebugBuiltInTracerType::CallTracer],
            expected[&GethDebugBuiltInTracerType::CallTracer]
        );
        assert_eq!(
            trace.0[&GethDebugBuiltInTracerType::PreStateTracer],
            expected[&GethDebugBuiltInTracerType::PreStateTracer]
        );
    }
}
