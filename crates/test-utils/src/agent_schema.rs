//! JSON Schema validation for agent-substrate `@v1` payloads.
//!
//! Loads the schema files committed under `docs/agents/schemas/` and exposes a
//! single [`validate`] entry point keyed by schema id. Test helpers wrap the
//! existing `machine_mode_*` snapshot assertions so envelope / stream payloads
//! are checked against the same schemas external agent tooling depends on.

use jsonschema::Validator;
use parking_lot::Mutex;
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, sync::OnceLock};

/// Returns the absolute path to `docs/agents/schemas/` in the workspace.
fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/agents/schemas")
}

/// Map of `schema_id` -> committed JSON Schema file name (relative to
/// `docs/agents/schemas/`).
const SCHEMA_FILES: &[(&str, &str)] = &[
    ("foundry:envelope@v1", "envelope.v1.json"),
    ("foundry:introspect@v1", "introspect.v1.json"),
    ("foundry:cast.call@v1", "cast.call.v1.json"),
    ("foundry:cast.abi-encode@v1", "cast.abi-encode.v1.json"),
    ("foundry:cast.abi-decode@v1", "cast.abi-decode.v1.json"),
    ("foundry:cast.keccak@v1", "cast.keccak.v1.json"),
    ("foundry:cast.4byte@v1", "cast.4byte.v1.json"),
    ("foundry:forge.build@v1", "forge.build.v1.json"),
    ("foundry:forge.create@v1", "forge.create.v1.json"),
    ("foundry:forge.test@v1", "forge.test.v1.json"),
    ("foundry:forge.test.event@v1", "forge.test.event.v1.json"),
    ("foundry:forge.script@v1", "forge.script.v1.json"),
    ("foundry:forge.script.event@v1", "forge.script.event.v1.json"),
];

/// The list of `@v1` schema ids covered by the agent-substrate freeze.
pub fn known_schema_ids() -> impl Iterator<Item = &'static str> {
    SCHEMA_FILES.iter().map(|(id, _)| *id)
}

fn validator_for(schema_id: &str) -> &'static Validator {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static Validator>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let (id, file) = SCHEMA_FILES
        .iter()
        .find(|(id, _)| *id == schema_id)
        .unwrap_or_else(|| panic!("unknown agent schema id: {schema_id}"));

    if let Some(v) = cache.lock().get(id) {
        return v;
    }

    let path = schemas_dir().join(file);
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let schema_value: Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    // `should_validate_formats(true)` opts in to hard-failing on bad
    // `format` values (e.g. `date-time`); the Draft 2020-12 default is
    // annotation-only.
    let validator: Validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema_value)
        .unwrap_or_else(|e| panic!("compile {}: {e}", path.display()));

    // `Validator` owns its compiled schema once built, so a single Box leak
    // is enough to give it a `'static` lifetime for the cache. The test
    // process exits clean these up.
    let leaked_validator: &'static Validator = Box::leak(Box::new(validator));
    cache.lock().insert(id, leaked_validator);
    leaked_validator
}

/// Validate `instance` against the committed schema for `schema_id`.
///
/// Panics with a descriptive message on validation failure. Use in tests to
/// pin emitted payloads to the same `@v1` contract external agent tooling
/// consumes.
pub fn validate(schema_id: &str, instance: &Value) {
    let validator = validator_for(schema_id);
    if let Err(error) = validator.validate(instance) {
        panic!(
            "schema `{schema_id}` validation failed: {error}\n\
             instance was:\n{}",
            serde_json::to_string_pretty(instance).unwrap_or_default()
        );
    }
}

/// Validate the `data` payload of an envelope-mode terminal envelope against
/// the per-command schema.
///
/// `envelope` is the parsed terminal envelope value (the last NDJSON line
/// under `--machine` stream mode, or the only line under envelope mode).
/// `data_schema_id` is the `result_schema_ref` reported in the introspect
/// registry for the command.
pub fn validate_envelope_data(envelope: &Value, data_schema_id: &str) {
    validate("foundry:envelope@v1", envelope);
    let data = envelope.get("data").unwrap_or_else(|| {
        panic!("envelope has no `data` field:\n{}", serde_json::to_string_pretty(envelope).unwrap())
    });
    assert!(!data.is_null(), "envelope `data` is null; cannot validate against {data_schema_id}");
    validate(data_schema_id, data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every committed schema must be a valid JSON Schema Draft 2020-12 document.
    #[test]
    fn all_committed_schemas_compile() {
        for id in known_schema_ids() {
            let _ = validator_for(id);
        }
    }

    #[test]
    fn envelope_minimal_success_validates() {
        let v = json!({
            "schema_version": 1,
            "success": true,
            "data": {},
            "errors": [],
            "warnings": []
        });
        validate("foundry:envelope@v1", &v);
    }

    #[test]
    fn envelope_failure_with_typed_error_validates() {
        let v = json!({
            "schema_version": 1,
            "success": false,
            "data": null,
            "errors": [{
                "level": "error",
                "code": "script.broadcast_failed",
                "message": "broadcast failed: insufficient funds",
            }],
            "warnings": []
        });
        validate("foundry:envelope@v1", &v);
    }

    #[test]
    fn envelope_rejects_malformed_diagnostic_code() {
        let v = json!({
            "schema_version": 1,
            "success": false,
            "data": null,
            "errors": [{ "level": "error", "code": "NotADotted", "message": "x" }],
            "warnings": []
        });
        let validator = validator_for("foundry:envelope@v1");
        assert!(validator.validate(&v).is_err());
    }

    #[test]
    fn forge_script_payload_validates() {
        let data = json!({
            "mode": "broadcast",
            "broadcast": true,
            "tx_count": 1,
            "tx_hashes": [
                "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            ],
            "created_contracts": [{
                "address": "0x000000000000000000000000000000000000beef",
                "contract_name": "Deployed",
            }]
        });
        validate("foundry:forge.script@v1", &data);
    }

    #[test]
    fn forge_test_event_oneof_kinds_validate() {
        for v in [
            json!({
                "schema_id": "foundry:forge.test.event@v1",
                "command_id": "forge.test",
                "kind": "test_result",
                "ts": "2025-01-01T00:00:00Z",
                "contract": "C.t.sol:CT",
                "name": "test_x",
                "status": "passed",
                "duration_ms": 1,
            }),
            json!({
                "schema_id": "foundry:forge.test.event@v1",
                "command_id": "forge.test",
                "kind": "suite_finished",
                "ts": "2025-01-01T00:00:00Z",
                "contract": "C.t.sol:CT",
                "passed": 1,
                "failed": 0,
                "skipped": 0,
                "duration_ms": 1,
            }),
            json!({
                "schema_id": "foundry:forge.test.event@v1",
                "command_id": "forge.test",
                "kind": "warning",
                "ts": "2025-01-01T00:00:00Z",
                "contract": "C.t.sol:CT",
                "code": "test.warning",
                "message": "x",
            }),
        ] {
            validate("foundry:forge.test.event@v1", &v);
        }
    }
}
