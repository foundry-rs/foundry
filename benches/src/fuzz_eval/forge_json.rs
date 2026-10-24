//! Parsing of `forge test --json` output.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Outcome of one test function as reported by `forge test --json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestRecord {
    /// Contract name, without the source path.
    pub contract: String,
    /// Test function name, without the parameter list.
    pub name: String,
    /// `Success`, `Failure`, or `Skipped`.
    pub status: String,
    /// Test kind (`Unit`, `Fuzz`, `Invariant`, ...), if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Test duration in seconds, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}

impl TestRecord {
    /// Returns whether the test failed.
    pub fn failed(&self) -> bool {
        self.status == "Failure"
    }

    /// Returns whether `pattern` (`test`, `Contract::test`, or a handler call name such as
    /// `Handler::withdraw`) names this test.
    pub fn matches(&self, pattern: &str) -> bool {
        let pattern = strip_params(pattern);
        if pattern == self.name {
            return true;
        }
        match pattern.split_once("::") {
            Some((contract, name)) => contract == self.contract && name == self.name,
            None => pattern == self.name,
        }
    }
}

/// Parses `forge test --json` stdout into per-test records sorted by contract and name.
///
/// Invariant campaigns are expanded so that each predicate and each handler assertion failure can
/// be matched individually: predicates reported through `invariant_predicate_results` or
/// `invariant_failures` become records named after the predicate, and entries of
/// `invariant_handler_failures` become failed records named after the failing handler call (for
/// example `Handler::withdraw`). Returns `None` when stdout contains no JSON object in the expected
/// shape.
pub fn parse_test_output(stdout: &str) -> Option<Vec<TestRecord>> {
    let value = parse_json_object(stdout)?;
    let suites = value.as_object()?;
    let mut records = Vec::new();
    for (suite_id, suite) in suites {
        let contract = suite_id.rsplit(':').next().unwrap_or(suite_id).to_string();
        let tests = suite.get("test_results")?.as_object()?;
        for (signature, result) in tests {
            let kind = result
                .get("kind")
                .and_then(Value::as_object)
                .and_then(|kind| kind.keys().next().cloned());
            let duration_secs =
                result.get("duration").and_then(Value::as_str).and_then(parse_duration_secs);
            let record = |name: &str, status: &str, kind: Option<String>| TestRecord {
                contract: contract.clone(),
                name: strip_params(name).to_string(),
                status: status.to_string(),
                kind,
                duration_secs,
            };
            let array = |key: &str| {
                result.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
            };

            let mut predicates = BTreeMap::new();
            for predicate in array("invariant_predicate_results") {
                if let Some(name) = predicate.get("name").and_then(Value::as_str) {
                    let status =
                        predicate.get("status").and_then(Value::as_str).unwrap_or("Failure");
                    predicates.insert(strip_params(name).to_string(), status.to_string());
                }
            }
            let mut handler_failures = 0;
            for failure in
                array("invariant_failures").iter().chain(array("invariant_handler_failures"))
            {
                let Some(name) = failure.get("name").and_then(Value::as_str) else { continue };
                if failure.get("kind").and_then(Value::as_str) == Some("handler") {
                    handler_failures += 1;
                    records.push(record(
                        &strip_source_path(name),
                        "Failure",
                        Some("Handler".to_string()),
                    ));
                } else {
                    predicates.insert(strip_params(name).to_string(), "Failure".to_string());
                }
            }

            let name = strip_params(signature).to_string();
            let mut status =
                result.get("status").and_then(Value::as_str).unwrap_or("Failure").to_string();
            if let Some(predicate_status) = predicates.remove(&name) {
                status = predicate_status;
            } else if handler_failures > 0 {
                // The campaign failed because of handler assertions, not this predicate.
                status = "Success".to_string();
            }
            for (predicate, predicate_status) in predicates {
                records.push(record(&predicate, &predicate_status, kind.clone()));
            }
            records.push(record(&name, &status, kind));
        }
    }
    records.sort_by(|a, b| (&a.contract, &a.name).cmp(&(&b.contract, &b.name)));
    records.dedup_by(|a, b| a.contract == b.contract && a.name == b.name);
    Some(records)
}

/// Parses a duration in jiff's friendly format (for example `1s 250ms 3µs`) into seconds.
pub fn parse_duration_secs(value: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut parsed_any = false;
    for token in value.split_whitespace() {
        let split = token.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let (number, unit) = token.split_at(split);
        let number = number.parse::<f64>().ok()?;
        let scale = match unit {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 1e-3,
            "µs" | "us" => 1e-6,
            "ns" => 1e-9,
            _ => return None,
        };
        total = number.mul_add(scale, total);
        parsed_any = true;
    }
    parsed_any.then_some(total)
}

/// Parses the whole output as JSON, falling back to the last line that is a JSON object.
fn parse_json_object(stdout: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(stdout.trim())
        && value.is_object()
    {
        return Some(value);
    }
    stdout
        .lines()
        .rev()
        .filter(|line| line.trim_start().starts_with('{'))
        .find_map(|line| serde_json::from_str::<Value>(line).ok().filter(Value::is_object))
}

/// Turns `path/File.sol:Contract::function` into `Contract::function`.
fn strip_source_path(name: &str) -> String {
    match name.split_once("::") {
        Some((contract, function)) => {
            format!("{}::{function}", contract.rsplit(':').next().unwrap_or(contract))
        }
        None => name.to_string(),
    }
}

fn strip_params(name: &str) -> &str {
    name.split_once('(').map_or(name, |(name, _)| name).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = r#"{"test/Vault.t.sol:VaultTest":{"duration":"1s 2ms","test_results":{
"testFuzz_deposit(uint256)":{"status":"Success","duration":"12ms 500µs","kind":{"Fuzz":{"runs":256}}},
"invariant_solvent()":{"status":"Failure","duration":"1s 500ms","kind":{"Invariant":{"runs":3}},
 "invariant_predicate_results":[{"name":"invariant_solvent","status":"Failure"},{"name":"invariant_supply()","status":"Success"}]}
},"warnings":[]},
"test/Escrow.t.sol:EscrowTest":{"duration":"3ms","test_results":{"test_unit()":{"status":"Skipped","duration":"0s","kind":{"Unit":{"gas":1}}}},"warnings":[]},
"test/recon/CryticToFoundry.sol:CryticToFoundry":{"duration":"2s","test_results":{
"invariant_noop()":{"status":"Failure","duration":"2s","kind":{"Invariant":{"runs":1}},
 "invariant_failures":[{"kind":"predicate","name":"invariant_solvency","reason":"x","persisted_path":"p"}],
 "invariant_handler_failures":[{"kind":"handler","name":"tests/recon/TargetFunctions.sol:TargetFunctions::spoke_withdraw","reason":"assert"}]}
},"warnings":[]}}"#;

    fn record(contract: &str, name: &str, status: &str, kind: &str, secs: f64) -> TestRecord {
        TestRecord {
            contract: contract.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            kind: Some(kind.to_string()),
            duration_secs: Some(secs),
        }
    }

    #[test]
    fn parses_suites_and_invariant_predicates() {
        let mut records = parse_test_output(OUTPUT).unwrap();
        for record in &mut records {
            record.duration_secs = record.duration_secs.map(|secs| (secs * 1e6).round() / 1e6);
        }
        assert_eq!(
            records,
            [
                record(
                    "CryticToFoundry",
                    "TargetFunctions::spoke_withdraw",
                    "Failure",
                    "Handler",
                    2.0
                ),
                record("CryticToFoundry", "invariant_noop", "Success", "Invariant", 2.0),
                record("CryticToFoundry", "invariant_solvency", "Failure", "Invariant", 2.0),
                record("EscrowTest", "test_unit", "Skipped", "Unit", 0.0),
                record("VaultTest", "invariant_solvent", "Failure", "Invariant", 1.5),
                record("VaultTest", "invariant_supply", "Success", "Invariant", 1.5),
                record("VaultTest", "testFuzz_deposit", "Success", "Fuzz", 0.0125),
            ]
        );
    }

    #[test]
    fn finds_json_after_other_output() {
        let stdout = format!("Compiling...\nnot json {{\n{}\n", OUTPUT.replace('\n', ""));
        assert_eq!(parse_test_output(&stdout).unwrap().len(), 7);
    }

    #[test]
    fn rejects_malformed_output() {
        assert_eq!(parse_test_output(""), None);
        assert_eq!(parse_test_output("error: compilation failed"), None);
        assert_eq!(parse_test_output(r#"{"suite":{"no_results":true}}"#), None);
    }

    #[test]
    fn matches_plain_and_qualified_names() {
        let test = record("VaultTest", "invariant_solvent", "Failure", "Invariant", 1.0);
        assert!(test.matches("invariant_solvent"));
        assert!(test.matches("invariant_solvent()"));
        assert!(test.matches("VaultTest::invariant_solvent"));
        assert!(!test.matches("OtherTest::invariant_solvent"));
        assert!(!test.matches("invariant_supply"));
        let handler = record("CryticToFoundry", "Handler::withdraw", "Failure", "Handler", 1.0);
        assert!(handler.matches("Handler::withdraw"));
        assert!(handler.matches("CryticToFoundry::Handler::withdraw"));
    }

    #[test]
    fn parses_friendly_durations() {
        assert_eq!(parse_duration_secs("0s"), Some(0.0));
        assert_eq!(parse_duration_secs("1m 30s"), Some(90.0));
        let secs = parse_duration_secs("1s 234ms 567µs 890ns").unwrap();
        assert!((secs - 1.234_567_89).abs() < 1e-12);
        assert_eq!(parse_duration_secs("soon"), None);
        assert_eq!(parse_duration_secs(""), None);
    }
}
