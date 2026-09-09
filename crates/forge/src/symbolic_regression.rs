use crate::result::{
    SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA, SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA_VERSION,
    SuiteResult, SymbolicArtifactRef, SymbolicCounterexampleArtifact,
    SymbolicCounterexampleArtifactKind, SymbolicRegressionRef, SymbolicReplayStatus,
};
use alloy_primitives::{U256, hex};
use eyre::{Result, bail};
use foundry_common::{TestFunctionExt, contracts::ContractsByArtifact, fs, sh_warn};
use foundry_config::Config;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    path::{Component, Path, PathBuf},
};

pub(crate) const SYMBOLIC_REGRESSION_MARKER: &str = "__foundry_symbolic_regression_marker";

/// Configuration for emitting Solidity regression tests from symbolic counterexamples.
#[derive(Clone, Debug)]
pub(crate) struct SymbolicRegressionConfig {
    pub out: Option<PathBuf>,
    pub overwrite: bool,
}

/// Solidity regression file emitted from a symbolic counterexample artifact.
#[derive(Clone, Debug)]
pub(crate) struct SymbolicRegression {
    pub artifact: PathBuf,
    pub path: PathBuf,
}

struct PlannedSymbolicRegression {
    regression: SymbolicRegression,
    contents: String,
    test: String,
}

pub(crate) fn emit_symbolic_regressions(
    config: &Config,
    regression: &SymbolicRegressionConfig,
    known_contracts: &ContractsByArtifact,
    results: &[SymbolicArtifactRef],
) -> Result<Vec<SymbolicRegression>> {
    let mut planned = Vec::new();
    let mut seen_tests = HashSet::new();
    for artifact_ref in results {
        let artifact = load_artifact(&artifact_ref.path)?;
        let (_, contract) = artifact_source_and_contract(&artifact_ref.path, &artifact)?;
        if contract.ends_with("_SymbolicRegression") {
            continue;
        }
        let key = (
            artifact.test.contract.clone(),
            artifact.test.test.clone(),
            match artifact.kind {
                SymbolicCounterexampleArtifactKind::SingleCall => "single_call",
                SymbolicCounterexampleArtifactKind::Sequence => "sequence",
            },
        );
        let suffix = if seen_tests.insert(key) {
            None
        } else if let Some(suffix) = handler_regression_suffix(&artifact_ref.path) {
            Some(suffix)
        } else {
            continue;
        };
        planned.push(plan_symbolic_regression(
            config,
            regression,
            &artifact_ref.path,
            &artifact,
            known_contracts,
            suffix.as_deref(),
        )?);
    }

    ensure_unique_regression_paths(&planned)?;

    let mut emitted = Vec::new();
    for plan in planned {
        if let Some(regression) = write_symbolic_regression(regression, plan)? {
            emitted.push(regression);
        }
    }
    Ok(emitted)
}

fn artifact_source_and_contract<'a>(
    artifact_path: &Path,
    artifact: &'a SymbolicCounterexampleArtifact,
) -> Result<(&'a str, &'a str)> {
    let Some((source, contract)) = artifact.test.contract.rsplit_once(':') else {
        bail!(
            "symbolic counterexample artifact {} test.contract must be `path:Contract`, got `{}`",
            artifact_path.display(),
            artifact.test.contract
        );
    };
    if source.is_empty() || contract.is_empty() {
        bail!(
            "symbolic counterexample artifact {} test.contract must be `path:Contract`, got `{}`",
            artifact_path.display(),
            artifact.test.contract
        );
    }
    Ok((source, contract))
}

pub(crate) fn collect_symbolic_artifacts_from_suites<'a>(
    suites: impl IntoIterator<Item = &'a SuiteResult>,
) -> Vec<SymbolicArtifactRef> {
    let mut artifacts = Vec::new();
    for suite in suites {
        for result in suite.test_results.values() {
            for artifact in &result.counterexample_artifacts {
                if !artifacts.contains(artifact) {
                    artifacts.push(artifact.clone());
                }
            }
        }
    }
    artifacts
}

pub(crate) fn attach_symbolic_regressions_to_suites<'a>(
    suites: impl IntoIterator<Item = &'a mut SuiteResult>,
    regressions: &[SymbolicRegression],
) {
    for suite in suites {
        for result in suite.test_results.values_mut() {
            for artifact in &result.counterexample_artifacts {
                for regression in regressions {
                    if artifact.path == regression.artifact
                        && !result
                            .symbolic_regressions
                            .iter()
                            .any(|existing| existing.path == regression.path)
                    {
                        result.symbolic_regressions.push(SymbolicRegressionRef {
                            artifact: regression.artifact.clone(),
                            path: regression.path.clone(),
                        });
                    }
                }
            }
        }
    }
}

fn load_artifact(path: &Path) -> Result<SymbolicCounterexampleArtifact> {
    let value = fs::read_json_file::<serde_json::Value>(path)?;
    let schema_version =
        value.get("schema_version").and_then(serde_json::Value::as_u64).ok_or_else(|| {
            eyre::eyre!(
                "symbolic counterexample artifact {} is missing numeric schema_version",
                path.display()
            )
        })?;
    if schema_version != u64::from(SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA_VERSION) {
        bail!(
            "unsupported symbolic counterexample artifact schema version {} in {}",
            schema_version,
            path.display()
        );
    }
    let schema = value.get("schema").and_then(serde_json::Value::as_str).ok_or_else(|| {
        eyre::eyre!("symbolic counterexample artifact {} is missing string schema", path.display())
    })?;
    if schema != SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA {
        bail!(
            "unsupported symbolic counterexample artifact schema `{}` in {}",
            schema,
            path.display()
        );
    }
    let artifact = serde_json::from_value::<SymbolicCounterexampleArtifact>(value)?;
    if artifact.replay.status != SymbolicReplayStatus::Confirmed {
        bail!(
            "symbolic counterexample artifact {} replay status must be confirmed, got {:?}",
            path.display(),
            artifact.replay.status
        );
    }
    if artifact.calls.is_empty() {
        bail!("symbolic counterexample artifact {} has no calls", path.display());
    }
    Ok(artifact)
}

fn plan_symbolic_regression(
    config: &Config,
    regression: &SymbolicRegressionConfig,
    artifact_path: &Path,
    artifact: &SymbolicCounterexampleArtifact,
    known_contracts: &ContractsByArtifact,
    suffix: Option<&str>,
) -> Result<PlannedSymbolicRegression> {
    let (source, contract) = artifact_source_and_contract(artifact_path, artifact)?;

    let test_name =
        artifact.test.test.split_once('(').map_or(artifact.test.test.as_str(), |(name, _)| name);
    let contract_ident = sanitize_identifier(contract);
    let test_ident = sanitize_identifier(test_name);
    let generated_contract = suffix.map_or_else(
        || format!("{contract_ident}_{test_ident}_SymbolicRegression"),
        |suffix| format!("{contract_ident}_{test_ident}_{suffix}_SymbolicRegression"),
    );
    let vm_interface = format!("{generated_contract}_Vm");
    let generated_test = format!("test_regression_{test_ident}_symbolic");
    let default_path = config.test.join("regressions").join(format!("{generated_contract}.t.sol"));
    let path = regression.out.as_ref().map_or(default_path, |out| {
        let out_is_file = if out.exists() {
            !out.is_dir()
        } else {
            out.extension().is_some_and(|ext| ext == std::ffi::OsStr::new("sol"))
        };
        if out_is_file { out.clone() } else { out.join(format!("{generated_contract}.t.sol")) }
    });

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let import_path =
        relative_solidity_import(path.parent().unwrap_or(&config.root), &config.root.join(source));
    let mut contents = String::new();
    writeln!(contents, "// SPDX-License-Identifier: UNLICENSED")?;
    writeln!(contents, "pragma solidity >=0.8.0;")?;
    writeln!(contents)?;
    writeln!(contents, "import \"{import_path}\";")?;
    writeln!(contents)?;
    writeln!(contents, "interface {vm_interface} {{")?;
    writeln!(contents, "    function deal(address who, uint256 newBalance) external;")?;
    writeln!(contents, "    function prank(address msgSender) external;")?;
    writeln!(contents, "    function roll(uint256 newHeight) external;")?;
    writeln!(contents, "    function warp(uint256 newTimestamp) external;")?;
    writeln!(contents, "}}")?;
    writeln!(contents)?;
    writeln!(contents, "contract {generated_contract} is {contract} {{")?;
    writeln!(
        contents,
        "    {vm_interface} private constant __foundrySymbolicVm = {vm_interface}(address(uint160(uint256(keccak256(\"hevm cheat code\")))));"
    )?;
    writeln!(contents)?;
    writeln!(
        contents,
        "    function {SYMBOLIC_REGRESSION_MARKER}() external pure returns (bytes32) {{"
    )?;
    writeln!(contents, "        return keccak256(\"foundry.symbolic.regression\");")?;
    writeln!(contents, "    }}")?;
    writeln!(contents)?;
    writeln!(contents, "    function {generated_test}() public payable {{")?;
    match artifact.kind {
        SymbolicCounterexampleArtifactKind::SingleCall => {
            let call = artifact.calls.first().expect("single-call artifact has at least one call");
            write_call(&mut contents, call, "address(this)", true, None)?;
        }
        SymbolicCounterexampleArtifactKind::Sequence => {
            let call_after_invariant = call_after_invariant(known_contracts, artifact)?;
            for (index, call) in artifact.calls.iter().enumerate() {
                write_call(
                    &mut contents,
                    call,
                    &format!("address({})", call.target),
                    artifact.replay_semantics.fail_on_revert,
                    Some(index),
                )?;
            }
            writeln!(
                contents,
                "        __foundrySymbolicRegressionCall(address(this), hex\"{}\", 0, true);",
                hex::encode(selector_from_signature(&artifact.test.test))
            )?;
            if call_after_invariant {
                writeln!(
                    contents,
                    "        __foundrySymbolicRegressionCall(address(this), hex\"{}\", 0, true);",
                    hex::encode(selector_from_signature("afterInvariant()"))
                )?;
            }
        }
    }
    writeln!(contents, "    }}")?;
    writeln!(contents)?;
    writeln!(
        contents,
        "    function __foundrySymbolicRegressionCall(address target, bytes memory data, uint256 value, bool bubbleFailure) internal returns (bool) {{"
    )?;
    writeln!(contents, "        if (target != address(this)) {{")?;
    writeln!(contents, "            uint256 codeSize;")?;
    writeln!(contents, "            assembly {{ codeSize := extcodesize(target) }}")?;
    writeln!(
        contents,
        "            require(codeSize != 0, \"symbolic regression target has no code\");"
    )?;
    writeln!(contents, "        }}")?;
    writeln!(contents, "        (bool ok, bytes memory ret) = target.call{{value: value}}(data);")?;
    writeln!(contents, "        if (!ok && bubbleFailure) {{")?;
    writeln!(contents, "            assembly {{")?;
    writeln!(contents, "                revert(add(ret, 0x20), mload(ret))")?;
    writeln!(contents, "            }}")?;
    writeln!(contents, "        }}")?;
    writeln!(contents, "        return ok;")?;
    writeln!(contents, "    }}")?;
    writeln!(contents, "}}")?;

    Ok(PlannedSymbolicRegression {
        regression: SymbolicRegression { artifact: artifact_path.to_path_buf(), path },
        contents,
        test: format!("{}::{}", artifact.test.contract, artifact.test.test),
    })
}

fn ensure_unique_regression_paths(planned: &[PlannedSymbolicRegression]) -> Result<()> {
    let mut seen = HashMap::<&Path, &PlannedSymbolicRegression>::new();
    for plan in planned {
        if let Some(previous) = seen.insert(&plan.regression.path, plan) {
            bail!(
                "multiple symbolic regressions resolve to {}; {} and {} cannot share one output file",
                plan.regression.path.display(),
                previous.test,
                plan.test
            );
        }
    }
    Ok(())
}

fn write_symbolic_regression(
    regression: &SymbolicRegressionConfig,
    plan: PlannedSymbolicRegression,
) -> Result<Option<SymbolicRegression>> {
    let SymbolicRegression { artifact, path } = &plan.regression;
    if path.exists() && !regression.overwrite {
        if std::fs::read_to_string(path).is_ok_and(|existing| existing == plan.contents) {
            return Ok(Some(plan.regression));
        }
        sh_warn!(
            "Regression test {} already exists; skipping {} (pass --regression-overwrite to replace it)",
            path.display(),
            artifact.display()
        )?;
        return Ok(None);
    }

    std::fs::write(path, plan.contents)?;
    Ok(Some(plan.regression))
}

fn write_call(
    out: &mut String,
    call: &crate::result::SymbolicCounterexampleCall,
    target: &str,
    bubble_failure: bool,
    sequence_call_index: Option<usize>,
) -> Result<()> {
    if let Some(warp) = call.warp {
        writeln!(
            out,
            "        __foundrySymbolicVm.warp(block.timestamp + {});",
            u256_literal(warp)
        )?;
    }
    if let Some(roll) = call.roll {
        writeln!(out, "        __foundrySymbolicVm.roll(block.number + {});", u256_literal(roll))?;
    }
    if let Some(value) = call.value
        && !value.is_zero()
    {
        if let Some(index) = sequence_call_index {
            writeln!(
                out,
                "        uint256 __foundrySymbolicValue{index} = {} <= address({}).balance ? {} : address({}).balance;",
                u256_literal(value),
                call.sender,
                u256_literal(value),
                call.sender
            )?;
            writeln!(
                out,
                "        if (__foundrySymbolicValue{index} != 0 && {} != address(this)) {{",
                call.sender
            )?;
            writeln!(
                out,
                "            __foundrySymbolicVm.deal(address(this), address(this).balance + __foundrySymbolicValue{index});"
            )?;
            writeln!(
                out,
                "            __foundrySymbolicVm.deal({}, address({}).balance - __foundrySymbolicValue{index});",
                call.sender, call.sender
            )?;
            writeln!(out, "        }}")?;
        } else {
            writeln!(
                out,
                "        __foundrySymbolicVm.deal(address(this), {});",
                u256_literal(value)
            )?;
            writeln!(
                out,
                "        __foundrySymbolicVm.deal({}, {});",
                call.sender,
                u256_literal(value)
            )?;
        }
    }
    writeln!(out, "        __foundrySymbolicVm.prank({});", call.sender)?;
    let value = sequence_call_index
        .filter(|_| call.value.is_some_and(|value| !value.is_zero()))
        .map_or_else(
            || call.value.map_or_else(|| "0".to_string(), u256_literal),
            |index| format!("__foundrySymbolicValue{index}"),
        );
    if let Some(index) = sequence_call_index {
        writeln!(
            out,
            "        bool __foundrySymbolicOk{index} = __foundrySymbolicRegressionCall({target}, hex\"{}\", {}, {});",
            hex::encode(call.calldata.as_ref()),
            value,
            if bubble_failure { "true" } else { "false" }
        )?;
        if call.value.is_some_and(|value| !value.is_zero()) {
            writeln!(
                out,
                "        if (__foundrySymbolicValue{index} != 0 && {} != address(this)) {{",
                call.sender
            )?;
            writeln!(out, "            if (!__foundrySymbolicOk{index}) {{")?;
            writeln!(
                out,
                "                __foundrySymbolicVm.deal({}, address({}).balance + __foundrySymbolicValue{index});",
                call.sender, call.sender
            )?;
            writeln!(
                out,
                "                __foundrySymbolicVm.deal(address(this), address(this).balance - __foundrySymbolicValue{index});"
            )?;
            writeln!(out, "            }}")?;
            writeln!(out, "        }}")?;
        }
    } else {
        writeln!(
            out,
            "        __foundrySymbolicRegressionCall({target}, hex\"{}\", {}, {});",
            hex::encode(call.calldata.as_ref()),
            value,
            if bubble_failure { "true" } else { "false" }
        )?;
    }
    Ok(())
}

fn selector_from_signature(signature: &str) -> [u8; 4] {
    let hash = alloy_primitives::keccak256(signature.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

fn u256_literal(value: U256) -> String {
    if value <= U256::from(u128::MAX) { value.to_string() } else { format!("0x{value:x}") }
}

fn sanitize_identifier(value: &str) -> String {
    let mut ident = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ident.push(ch);
        } else {
            ident.push('_');
        }
    }
    if ident.is_empty() || ident.as_bytes()[0].is_ascii_digit() {
        ident.insert(0, '_');
    }
    ident
}

fn handler_regression_suffix(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    stem.starts_with("handler-").then(|| sanitize_identifier(&stem))
}

fn call_after_invariant(
    known_contracts: &ContractsByArtifact,
    artifact: &SymbolicCounterexampleArtifact,
) -> Result<bool> {
    let Some((_, contract)) =
        known_contracts.find_by_name_or_identifier(&artifact.test.contract)?
    else {
        return Ok(false);
    };
    let mut after_invariant_fns =
        contract.abi.functions().filter(|func| func.name.is_after_invariant());
    let Some(after_invariant) = after_invariant_fns.next() else {
        return Ok(false);
    };
    Ok(after_invariant.name == "afterInvariant" && after_invariant_fns.next().is_none())
}

fn relative_solidity_import(from_dir: &Path, to_file: &Path) -> String {
    let from = std::fs::canonicalize(from_dir).unwrap_or_else(|_| from_dir.to_path_buf());
    let to = std::fs::canonicalize(to_file).unwrap_or_else(|_| to_file.to_path_buf());
    let from = normalized_components(&from);
    let to = normalized_components(&to);
    let shared = from.iter().zip(&to).take_while(|(left, right)| left == right).count();
    let mut parts = Vec::new();
    parts.extend(std::iter::repeat_n("..".to_string(), from.len().saturating_sub(shared)));
    parts.extend(to[shared..].iter().cloned());
    if parts.is_empty() {
        ".".to_string()
    } else {
        let path = parts.join("/");
        if path.starts_with('.') { path } else { format!("./{path}") }
    }
}

fn normalized_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            Component::ParentDir => Some("..".to_string()),
            Component::CurDir => None,
            Component::RootDir | Component::Prefix(_) => {
                Some(component.as_os_str().to_string_lossy().to_string())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::{
        SymbolicCallTrace, SymbolicCounterexample, SymbolicCounterexampleReplaySemantics,
        SymbolicCounterexampleTestIdentity, SymbolicReplayMetadata, SymbolicResult,
    };
    use alloy_primitives::{Address, Bytes};
    use foundry_config::SymbolicConfig;
    use foundry_evm_symbolic::SymbolicStats;

    fn config(root: &Path) -> Config {
        Config { root: root.to_path_buf(), test: root.join("test"), ..Default::default() }
    }

    fn artifact(
        kind: SymbolicCounterexampleArtifactKind,
        calls: Vec<crate::result::SymbolicCounterexampleCall>,
    ) -> SymbolicCounterexampleArtifact {
        let call = calls.first().expect("artifact has at least one call");
        let symbolic = SymbolicResult::fail_counterexample(
            &SymbolicConfig::default(),
            SymbolicStats::default(),
            SymbolicCallTrace::none(),
            SymbolicCounterexample {
                calldata: call.calldata.clone(),
                args: None,
                raw_args: None,
                value: call.value,
            },
        );
        let mut artifact = SymbolicCounterexampleArtifact::new(
            kind,
            SymbolicCounterexampleTestIdentity {
                contract: "test/Regression.t.sol:Regression".to_string(),
                test: "invariant_value()".to_string(),
            },
            &symbolic,
            SymbolicCounterexampleReplaySemantics { fail_on_revert: false },
            calls,
        );
        artifact.replay = SymbolicReplayMetadata::confirmed();
        artifact
    }

    fn value_call(target: Address) -> crate::result::SymbolicCounterexampleCall {
        crate::result::SymbolicCounterexampleCall {
            warp: None,
            roll: None,
            sender: Address::repeat_byte(0xb0),
            target,
            calldata: Bytes::from_static(&[0x12, 0x34, 0x56, 0x78]),
            value: Some(U256::from(10)),
            contract_name: None,
            function_name: None,
            signature: None,
            args: None,
            raw_args: None,
        }
    }

    fn plan(
        kind: SymbolicCounterexampleArtifactKind,
        calls: Vec<crate::result::SymbolicCounterexampleCall>,
    ) -> String {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("test/Regression.t.sol");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "contract Regression {}").unwrap();

        let artifact_path = temp.path().join("cache/artifact.json");
        let artifact = artifact(kind, calls);
        plan_symbolic_regression(
            &config(temp.path()),
            &SymbolicRegressionConfig { out: None, overwrite: false },
            &artifact_path,
            &artifact,
            &ContractsByArtifact::default(),
            None,
        )
        .unwrap()
        .contents
    }

    #[test]
    fn sequence_regression_clamps_and_debits_sender_balance() {
        let contents = plan(
            SymbolicCounterexampleArtifactKind::Sequence,
            vec![value_call(Address::repeat_byte(0x7f)), value_call(Address::repeat_byte(0x8f))],
        );
        let lowercase = contents.to_lowercase();

        assert!(lowercase.contains(
            "uint256 __foundrysymbolicvalue0 = 10 <= address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance ? 10 : address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance;"
        ));
        assert!(lowercase.contains(
            "uint256 __foundrysymbolicvalue1 = 10 <= address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance ? 10 : address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance;"
        ));
        assert!(contents.contains("if (__foundrySymbolicValue0 != 0"));
        assert!(contents.contains("if (__foundrySymbolicValue1 != 0"));
        assert!(contents.contains(
            "__foundrySymbolicVm.deal(address(this), address(this).balance + __foundrySymbolicValue0);"
        ));
        assert!(contents.contains("bool __foundrySymbolicOk0 = __foundrySymbolicRegressionCall"));
        assert!(contents.contains("if (!__foundrySymbolicOk0) {"));
        assert!(lowercase.contains(
            "__foundrysymbolicvm.deal(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0, address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance - __foundrysymbolicvalue0);"
        ));
        assert!(lowercase.contains(
            "__foundrysymbolicvm.deal(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0, address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance - __foundrysymbolicvalue1);"
        ));
        assert!(lowercase.contains(
            "__foundrysymbolicvm.deal(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0, address(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0).balance + __foundrysymbolicvalue0);"
        ));
        assert!(contents.contains(
            "__foundrySymbolicVm.deal(address(this), address(this).balance - __foundrySymbolicValue0);"
        ));
        assert!(lowercase.contains(
            "__foundrysymbolicregressioncall(address(0x7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f), hex\"12345678\", __foundrysymbolicvalue0, false);"
        ));
        assert!(lowercase.contains(
            "__foundrysymbolicregressioncall(address(0x8f8f8f8f8f8f8f8f8f8f8f8f8f8f8f8f8f8f8f8f), hex\"12345678\", __foundrysymbolicvalue1, false);"
        ));
        assert!(
            !lowercase.contains(
                "__foundrysymbolicvm.deal(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0, 10);"
            )
        );
    }

    #[test]
    fn single_call_regression_preserves_exact_value_funding() {
        let contents = plan(
            SymbolicCounterexampleArtifactKind::SingleCall,
            vec![value_call(Address::repeat_byte(0x7f))],
        );
        let lowercase = contents.to_lowercase();

        assert!(contents.contains("__foundrySymbolicVm.deal(address(this), 10);"));
        assert!(
            lowercase.contains(
                "__foundrysymbolicvm.deal(0xb0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0, 10);"
            )
        );
        assert!(contents.contains(
            "__foundrySymbolicRegressionCall(address(this), hex\"12345678\", 10, true);"
        ));
        assert!(!contents.contains("bool __foundrySymbolicOk0 ="));
    }

    #[test]
    fn relative_solidity_import_canonicalizes_symlinked_absolute_paths() {
        let tmp = Path::new("/tmp");
        let Ok(canonical_tmp) = std::fs::canonicalize(tmp) else { return };
        if canonical_tmp == tmp {
            return;
        }

        let root =
            tmp.join(format!("foundry-symbolic-regression-import-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let from_dir = root.join("test");
        std::fs::create_dir_all(&from_dir).unwrap();
        let to_file = from_dir.join("Original.t.sol");
        std::fs::write(&to_file, "contract Original {}").unwrap();
        let canonical_to_file = std::fs::canonicalize(&to_file).unwrap();

        let import = relative_solidity_import(&from_dir, &canonical_to_file);

        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(import, "./Original.t.sol");
    }
}
