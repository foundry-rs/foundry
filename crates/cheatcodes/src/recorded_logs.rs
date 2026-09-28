//! EVM log capture shared by cheatcode inspectors.

use crate::{Result, Vm};
use alloy_primitives::{Log, hex};
use alloy_sol_types::SolValue;
use serde::Serialize;

/// Appends a log while recording is active.
pub fn record(recorded: &mut Option<Vec<Vm::Log>>, log: &Log) {
    if let Some(recorded) = recorded {
        recorded.push(Vm::Log {
            topics: log.data.topics().to_vec(),
            data: log.data.data.clone(),
            emitter: log.address,
        });
    }
}

/// Returns the current recording and starts a fresh one.
pub fn take(recorded: &mut Option<Vec<Vm::Log>>) -> Vec<Vm::Log> {
    recorded.replace(Vec::new()).unwrap_or_default()
}

/// ABI-encodes a JSON string containing the current recording.
pub fn take_json(recorded: &mut Option<Vec<Vm::Log>>) -> Result {
    let logs = take(recorded)
        .into_iter()
        .map(|log| LogJson {
            topics: log.topics.iter().map(ToString::to_string).collect(),
            data: hex::encode_prefixed(&log.data),
            emitter: log.emitter.to_string(),
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string(&logs)?.abi_encode())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogJson {
    topics: Vec<String>,
    data: String,
    emitter: String,
}
