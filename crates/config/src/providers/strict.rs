//! Accepted TOML keys, including flattened fields and structured map values.
//!
//! Keep these definitions aligned with the corresponding serde wire types. Optional fields and
//! aliases belong here even when they are omitted from serialized defaults. Value deserialization
//! remains responsible for validating scalar types and values.

use crate::{Config, DEPRECATIONS, error::UNKNOWN_CONFIG_KEY_EXPECTED};
use figment::{Error, Profile, error::Kind};
use foundry_evm_networks::NetworkConfigs;
use heck::ToSnakeCase;
use std::path::Path;
use toml::Value;

#[derive(Clone, Copy)]
enum Schema {
    Value,
    Fields(&'static [(&'static str, Self)]),
    FieldsOrString(&'static [(&'static str, Self)]),
    Array(&'static Self),
    Map(&'static Self),
}

/// Validates every profile before selection or inheritance can discard unknown keys.
#[cfg(test)]
pub(super) fn validate_toml_keys(contents: &str, source: &Path) -> Result<(), Error> {
    validate_toml_keys_classified(contents, source).map_err(StrictValidationError::into_error)
}

pub(super) enum StrictValidationError {
    Provider(Error),
    UnknownKeys(Error),
}

#[cfg(test)]
impl StrictValidationError {
    fn into_error(self) -> Error {
        match self {
            Self::Provider(err) => err,
            Self::UnknownKeys(err) => {
                if let Kind::UnknownField(message, expected) = &err.kind
                    && *expected == UNKNOWN_CONFIG_KEY_EXPECTED
                {
                    Error::from(message.clone())
                } else {
                    err
                }
            }
        }
    }
}

pub(super) fn validate_toml_keys_classified(
    contents: &str,
    source: &Path,
) -> Result<(), StrictValidationError> {
    let source_path = source.to_string_lossy();
    let value = toml::from_str::<Value>(contents).map_err(|err| {
        StrictValidationError::Provider(Error::from(err.to_string()).with_path(&source_path))
    })?;
    let mut unknown = Vec::new();
    let mut invalid = Vec::new();
    if let Some(root) = value.as_table() {
        for (key, value) in root {
            let section = Profile::new(key);
            if section == Profile::new(Config::EXTERNAL_SECTION) {
                continue;
            }
            if section == Profile::new(Config::PROFILE_SECTION) {
                if let Some(profiles) = value.as_table() {
                    for (profile, value) in profiles {
                        validate_config(
                            value,
                            &format!("{key}.{profile}"),
                            &mut unknown,
                            &mut invalid,
                        );
                    }
                } else {
                    invalid.push(format!("{key} (expected table)"));
                }
            } else if Config::STANDALONE_SECTIONS.iter().any(|name| section == Profile::new(name)) {
                if let Some((_, schema)) =
                    CONFIG.iter().find(|(name, _)| section == Profile::new(name))
                {
                    validate(*schema, value, key, &mut unknown, &mut invalid);
                }
            } else if value.is_table() {
                // Before explicit `[profile.<name>]` tables were introduced, every non-standalone
                // top-level table was treated as a profile. Keep accepting and validating that
                // syntax so the provider can emit its migration warning.
                validate_config(value, key, &mut unknown, &mut invalid);
            } else {
                unknown.push(key.clone());
            }
        }
    }
    if !unknown.is_empty() {
        unknown.sort();
        return Err(StrictValidationError::UnknownKeys(Error::from(Kind::UnknownField(
            format!("Unknown configuration keys in {}: {}", source.display(), unknown.join(", ")),
            UNKNOWN_CONFIG_KEY_EXPECTED,
        ))));
    }
    if !invalid.is_empty() {
        invalid.sort();
        return Err(StrictValidationError::Provider(Error::from(format!(
            "Invalid configuration values in {}: {}",
            source.display(),
            invalid.join(", ")
        ))));
    }
    Ok(())
}

fn validate_config(
    value: &Value,
    path: &str,
    unknown: &mut Vec<String>,
    invalid: &mut Vec<String>,
) {
    if let Some(table) = value.as_table() {
        for (key, value) in table {
            let lookup = key.to_snake_case();
            let path = format!("{path}.{key}");
            if let Some((_, schema)) = CONFIG.iter().find(|(name, _)| *name == lookup) {
                validate(*schema, value, &path, unknown, invalid);
            } else if NetworkConfigs::is_config_key(&lookup) {
                // These fields are flattened into `Config` and vary with network crate features.
            } else if !DEPRECATIONS.iter().any(|(deprecated, _)| *deprecated == lookup) {
                unknown.push(path);
            }
        }
    } else {
        invalid.push(format!("{path} (expected table)"));
    }
}

fn validate(
    schema: Schema,
    value: &Value,
    path: &str,
    unknown: &mut Vec<String>,
    invalid: &mut Vec<String>,
) {
    match schema {
        Schema::Fields(fields) | Schema::FieldsOrString(fields) if value.is_table() => {
            if let Some(table) = value.as_table() {
                for (key, value) in table {
                    let path = format!("{path}.{key}");
                    if let Some((_, schema)) = fields.iter().find(|(name, _)| *name == key) {
                        validate(*schema, value, &path, unknown, invalid);
                    } else {
                        unknown.push(path);
                    }
                }
            }
        }
        Schema::Fields(_) => invalid.push(format!("{path} (expected table)")),
        Schema::FieldsOrString(_) if value.is_str() => {}
        Schema::FieldsOrString(_) => invalid.push(format!("{path} (expected table or string)")),
        Schema::Array(item) => {
            if let Some(array) = value.as_array() {
                for (index, value) in array.iter().enumerate() {
                    validate(*item, value, &format!("{path}[{index}]"), unknown, invalid);
                }
            } else {
                invalid.push(format!("{path} (expected array)"));
            }
        }
        Schema::Map(item) => {
            if let Some(table) = value.as_table() {
                for (key, value) in table {
                    validate(*item, value, &format!("{path}.{key}"), unknown, invalid);
                }
            } else {
                invalid.push(format!("{path} (expected table)"));
            }
        }
        Schema::Value => {}
    }
}

const CONFIG: &[(&str, Schema)] = &[
    ("root", Schema::Value),
    ("extends", Schema::FieldsOrString(EXTEND_CONFIG)),
    ("src", Schema::Value),
    ("test", Schema::Value),
    ("script", Schema::Value),
    ("out", Schema::Value),
    ("libs", Schema::Value),
    ("remappings", Schema::Value),
    ("auto_detect_remappings", Schema::Value),
    ("libraries", Schema::Value),
    ("cache", Schema::Value),
    ("cache_path", Schema::Value),
    ("dynamic_test_linking", Schema::Value),
    ("snapshots", Schema::Value),
    ("gas_snapshot_check", Schema::Value),
    ("gas_snapshot_emit", Schema::Value),
    ("broadcast", Schema::Value),
    ("allow_paths", Schema::Value),
    ("include_paths", Schema::Value),
    ("skip", Schema::Value),
    ("force", Schema::Value),
    ("evm_version", Schema::Value),
    ("hardfork", Schema::Value),
    ("gas_reports", Schema::Value),
    ("gas_reports_ignore", Schema::Value),
    ("gas_reports_include_tests", Schema::Value),
    ("solc", Schema::Value),
    ("auto_detect_solc", Schema::Value),
    ("offline", Schema::Value),
    ("optimizer", Schema::Value),
    ("optimizer_runs", Schema::Value),
    ("optimizer_details", Schema::Fields(OPTIMIZER_DETAILS)),
    ("model_checker", Schema::Fields(MODEL_CHECKER_SETTINGS)),
    ("verbosity", Schema::Value),
    ("eth_rpc_url", Schema::Value),
    ("eth_rpc_accept_invalid_certs", Schema::Value),
    ("eth_rpc_no_proxy", Schema::Value),
    ("eth_rpc_jwt", Schema::Value),
    ("eth_rpc_timeout", Schema::Value),
    ("eth_rpc_headers", Schema::Value),
    ("eth_rpc_curl", Schema::Value),
    ("etherscan_api_key", Schema::Value),
    ("etherscan", Schema::Map(&Schema::Fields(ETHERSCAN_CONFIG))),
    ("ignored_error_codes", Schema::Value),
    ("ignored_error_codes_from", Schema::Value),
    ("ignored_warnings_from", Schema::Value),
    ("deny", Schema::Value),
    ("deny_warnings", Schema::Value),
    ("match_test", Schema::Value),
    ("no_match_test", Schema::Value),
    ("match_contract", Schema::Value),
    ("no_match_contract", Schema::Value),
    ("match_path", Schema::Value),
    ("no_match_path", Schema::Value),
    ("no_match_coverage", Schema::Value),
    ("test_failures_file", Schema::Value),
    ("mutation_dir", Schema::Value),
    ("threads", Schema::Value),
    ("show_progress", Schema::Value),
    ("fuzz", Schema::Fields(FUZZ_CONFIG)),
    ("invariant", Schema::Fields(INVARIANT_CONFIG)),
    ("symbolic", Schema::Fields(SYMBOLIC_CONFIG)),
    ("coverage", Schema::Fields(COVERAGE_CONFIG)),
    ("mutation", Schema::Fields(MUTATION_CONFIG)),
    ("tracing", Schema::Fields(TRACING_CONFIG)),
    ("ffi", Schema::Value),
    ("live_logs", Schema::Value),
    ("allow_internal_expect_revert", Schema::Value),
    ("always_use_create_2_factory", Schema::Value),
    ("eip1559_fee_estimate", Schema::Value),
    ("prompt_timeout", Schema::Value),
    ("sender", Schema::Value),
    ("tx_origin", Schema::Value),
    ("initial_balance", Schema::Value),
    ("block_number", Schema::Value),
    ("fork_block_number", Schema::Value),
    ("chain_id", Schema::Value),
    ("chain", Schema::Value),
    ("gas_limit", Schema::Value),
    ("code_size_limit", Schema::Value),
    ("gas_price", Schema::Value),
    ("block_base_fee_per_gas", Schema::Value),
    ("block_coinbase", Schema::Value),
    ("block_timestamp", Schema::Value),
    ("block_difficulty", Schema::Value),
    ("block_prevrandao", Schema::Value),
    ("block_gas_limit", Schema::Value),
    ("memory_limit", Schema::Value),
    ("extra_output", Schema::Value),
    ("extra_output_files", Schema::Value),
    ("names", Schema::Value),
    ("sizes", Schema::Value),
    ("via_ir", Schema::Value),
    ("via_ssa_cfg", Schema::Value),
    ("experimental", Schema::Value),
    ("ast", Schema::Value),
    ("rpc_storage_caching", Schema::Fields(STORAGE_CACHING_CONFIG)),
    ("no_storage_caching", Schema::Value),
    ("no_rpc_rate_limit", Schema::Value),
    ("rpc_endpoints", Schema::Map(&Schema::FieldsOrString(RPC_ENDPOINT))),
    ("use_literal_content", Schema::Value),
    ("bytecode_hash", Schema::Value),
    ("cbor_metadata", Schema::Value),
    ("revert_strings", Schema::Value),
    ("sparse_mode", Schema::Value),
    ("build_info", Schema::Value),
    ("build_info_path", Schema::Value),
    ("fmt", Schema::Fields(FORMATTER_CONFIG)),
    ("lint", Schema::Fields(LINTER_CONFIG)),
    ("doc", Schema::Fields(DOC_CONFIG)),
    ("bind_json", Schema::Fields(BIND_JSON_CONFIG)),
    ("fs_permissions", Schema::Array(&Schema::Fields(PATH_PERMISSION))),
    ("isolate", Schema::Value),
    ("disable_block_gas_limit", Schema::Value),
    ("enable_tx_gas_limit", Schema::Value),
    ("labels", Schema::Value),
    ("unchecked_cheatcode_artifacts", Schema::Value),
    ("create2_library_salt", Schema::Value),
    ("create2_deployer", Schema::Value),
    ("vyper", Schema::Fields(VYPER_CONFIG)),
    ("dependencies", Schema::Map(&Schema::FieldsOrString(MAP_DEPENDENCY))),
    ("soldeer", Schema::Fields(SOLDEER_CONFIG)),
    ("assertions_revert", Schema::Value),
    ("legacy_assertions", Schema::Value),
    ("extra_args", Schema::Value),
    ("transaction_timeout", Schema::Value),
    ("additional_compiler_profiles", Schema::Array(&Schema::Fields(SETTINGS_OVERRIDES))),
    ("compilation_restrictions", Schema::Array(&Schema::Fields(COMPILATION_RESTRICTIONS))),
    ("script_execution_protection", Schema::Value),
    ("solc_version", Schema::Value),
];

const EXTEND_CONFIG: &[(&str, Schema)] = &[("path", Schema::Value), ("strategy", Schema::Value)];

const OPTIMIZER_DETAILS: &[(&str, Schema)] = &[
    ("peephole", Schema::Value),
    ("inliner", Schema::Value),
    ("jumpdestRemover", Schema::Value),
    ("orderLiterals", Schema::Value),
    ("deduplicate", Schema::Value),
    ("cse", Schema::Value),
    ("constantOptimizer", Schema::Value),
    ("yul", Schema::Value),
    ("yulDetails", Schema::Fields(YUL_DETAILS)),
    ("simpleCounterForLoopUncheckedIncrement", Schema::Value),
];

const YUL_DETAILS: &[(&str, Schema)] =
    &[("stackAllocation", Schema::Value), ("optimizerSteps", Schema::Value)];

const MODEL_CHECKER_SETTINGS: &[(&str, Schema)] = &[
    ("contracts", Schema::Value),
    ("engine", Schema::Value),
    ("timeout", Schema::Value),
    ("targets", Schema::Value),
    ("invariants", Schema::Value),
    ("showUnproved", Schema::Value),
    ("show_unproved", Schema::Value),
    ("divModWithSlacks", Schema::Value),
    ("div_mod_with_slacks", Schema::Value),
    ("solvers", Schema::Value),
    ("showUnsupported", Schema::Value),
    ("show_unsupported", Schema::Value),
    ("showProvedSafe", Schema::Value),
    ("show_proved_safe", Schema::Value),
];

const ETHERSCAN_CONFIG: &[(&str, Schema)] =
    &[("chain", Schema::Value), ("url", Schema::Value), ("key", Schema::Value)];

const FUZZ_CONFIG: &[(&str, Schema)] = &[
    ("runs", Schema::Value),
    ("run", Schema::Value),
    ("worker", Schema::Value),
    ("fail_on_revert", Schema::Value),
    ("max_test_rejects", Schema::Value),
    ("seed", Schema::Value),
    ("dictionary_weight", Schema::Value),
    ("include_storage", Schema::Value),
    ("include_push_bytes", Schema::Value),
    ("max_fuzz_dictionary_addresses", Schema::Value),
    ("max_fuzz_dictionary_values", Schema::Value),
    ("max_fuzz_dictionary_literals", Schema::Value),
    ("gas_report_samples", Schema::Value),
    ("corpus_dir", Schema::Value),
    ("frontier_dir", Schema::Value),
    ("frontier_limit", Schema::Value),
    ("corpus_gzip", Schema::Value),
    ("corpus_min_mutations", Schema::Value),
    ("corpus_min_size", Schema::Value),
    ("show_edge_coverage", Schema::Value),
    ("evm_edge_coverage_collision_free", Schema::Value),
    ("evm_edge_coverage_include_call_depth", Schema::Value),
    ("sancov_edges", Schema::Value),
    ("sancov_trace_cmp", Schema::Value),
    ("corpus_random_sequence_weight", Schema::Value),
    ("payable_value_weight", Schema::Value),
    ("mutation_weight_splice", Schema::Value),
    ("mutation_weight_repeat", Schema::Value),
    ("mutation_weight_interleave", Schema::Value),
    ("mutation_weight_prefix", Schema::Value),
    ("mutation_weight_suffix", Schema::Value),
    ("mutation_weight_abi", Schema::Value),
    ("mutation_weight_cmp", Schema::Value),
    ("failure_persist_dir", Schema::Value),
    ("show_logs", Schema::Value),
    ("timeout", Schema::Value),
];

const INVARIANT_CONFIG: &[(&str, Schema)] = &[
    ("runs", Schema::Value),
    ("depth", Schema::Value),
    ("min_depth", Schema::Value),
    ("depth_mode", Schema::Value),
    ("workers", Schema::Value),
    ("fail_on_revert", Schema::Value),
    ("call_override", Schema::Value),
    ("dictionary_weight", Schema::Value),
    ("include_storage", Schema::Value),
    ("include_push_bytes", Schema::Value),
    ("max_fuzz_dictionary_addresses", Schema::Value),
    ("max_fuzz_dictionary_values", Schema::Value),
    ("max_fuzz_dictionary_literals", Schema::Value),
    ("shrink_run_limit", Schema::Value),
    ("max_assume_rejects", Schema::Value),
    ("gas_report_samples", Schema::Value),
    ("corpus_dir", Schema::Value),
    ("frontier_dir", Schema::Value),
    ("frontier_limit", Schema::Value),
    ("corpus_gzip", Schema::Value),
    ("corpus_min_mutations", Schema::Value),
    ("corpus_min_size", Schema::Value),
    ("show_edge_coverage", Schema::Value),
    ("evm_edge_coverage_collision_free", Schema::Value),
    ("evm_edge_coverage_include_call_depth", Schema::Value),
    ("sancov_edges", Schema::Value),
    ("sancov_trace_cmp", Schema::Value),
    ("corpus_random_sequence_weight", Schema::Value),
    ("payable_value_weight", Schema::Value),
    ("mutation_weight_splice", Schema::Value),
    ("mutation_weight_repeat", Schema::Value),
    ("mutation_weight_interleave", Schema::Value),
    ("mutation_weight_prefix", Schema::Value),
    ("mutation_weight_suffix", Schema::Value),
    ("mutation_weight_abi", Schema::Value),
    ("mutation_weight_cmp", Schema::Value),
    ("corpus_random_sequence_weight_configured", Schema::Value),
    ("workers_configured", Schema::Value),
    ("failure_persist_dir", Schema::Value),
    ("show_metrics", Schema::Value),
    ("timeout", Schema::Value),
    ("show_solidity", Schema::Value),
    ("max_time_delay", Schema::Value),
    ("max_block_delay", Schema::Value),
    ("check_interval", Schema::Value),
];

const SYMBOLIC_CONFIG: &[(&str, Schema)] = &[
    ("enabled", Schema::Value),
    ("seed_corpus", Schema::Value),
    ("use_fuzz_corpus", Schema::Value),
    ("corpus_seed_limit", Schema::Value),
    ("use_fuzz_frontiers", Schema::Value),
    ("check_invariant_frontiers", Schema::Value),
    ("frontier_limit", Schema::Value),
    ("frontier_ids", Schema::Value),
    ("frontier_pcs", Schema::Value),
    ("frontier_selectors", Schema::Value),
    ("solver", Schema::Value),
    ("solver_command", Schema::Value),
    ("solver_portfolio", Schema::Value),
    ("timeout", Schema::Value),
    ("loop", Schema::Value),
    ("depth", Schema::Value),
    ("width", Schema::Value),
    ("max_depth", Schema::Value),
    ("max_paths", Schema::Value),
    ("invariant_depth", Schema::Value),
    ("exploration_order", Schema::Value),
    ("max_solver_queries", Schema::Value),
    ("default_dynamic_length", Schema::Value),
    ("max_dynamic_length", Schema::Value),
    ("array_lengths", Schema::Value),
    ("dynamic_lengths", Schema::Value),
    ("default_array_lengths", Schema::Value),
    ("default_bytes_lengths", Schema::Value),
    ("max_calldata_bytes", Schema::Value),
    ("symbolic_call_targets", Schema::Value),
    ("dump_smt", Schema::Value),
    ("storage_layout", Schema::Value),
];

const COVERAGE_CONFIG: &[(&str, Schema)] = &[
    ("report", Schema::Value),
    ("lcov_version", Schema::Value),
    ("ir_minimum", Schema::Value),
    ("report_file", Schema::Value),
    ("include_libs", Schema::Value),
    ("exclude_tests", Schema::Value),
    ("skip_files", Schema::Value),
];

const MUTATION_CONFIG: &[(&str, Schema)] = &[
    ("include_operators", Schema::Value),
    ("exclude_operators", Schema::Value),
    ("timeout", Schema::Value),
    ("optimizer_runs", Schema::Value),
    ("via_ir", Schema::Value),
];

const TRACING_CONFIG: &[(&str, Schema)] = &[
    ("verbosity", Schema::Value),
    ("labels", Schema::Value),
    ("disable_labels", Schema::Value),
    ("compact_labels", Schema::Value),
    ("trace_depth", Schema::Value),
    ("decode_internal", Schema::Value),
    ("external_identification_timeout", Schema::Value),
];

const STORAGE_CACHING_CONFIG: &[(&str, Schema)] =
    &[("chains", Schema::Value), ("endpoints", Schema::Value)];

const RPC_ENDPOINT: &[(&str, Schema)] = &[
    ("endpoint", Schema::Value),
    ("url", Schema::Value),
    ("endpoints", Schema::Value),
    ("retries", Schema::Value),
    ("retry_backoff", Schema::Value),
    ("compute_units_per_second", Schema::Value),
    ("auth", Schema::Value),
];

const FORMATTER_CONFIG: &[(&str, Schema)] = &[
    ("line_length", Schema::Value),
    ("tab_width", Schema::Value),
    ("style", Schema::Value),
    ("bracket_spacing", Schema::Value),
    ("int_types", Schema::Value),
    ("multiline_func_header", Schema::Value),
    ("quote_style", Schema::Value),
    ("number_underscore", Schema::Value),
    ("hex_underscore", Schema::Value),
    ("single_line_statement_blocks", Schema::Value),
    ("override_spacing", Schema::Value),
    ("wrap_comments", Schema::Value),
    ("docs_style", Schema::Value),
    ("ignore", Schema::Value),
    ("contract_new_lines", Schema::Value),
    ("sort_imports", Schema::Value),
    ("namespace_import_style", Schema::Value),
    ("pow_no_space", Schema::Value),
    ("prefer_compact", Schema::Value),
    ("single_line_imports", Schema::Value),
];

const LINTER_CONFIG: &[(&str, Schema)] = &[
    ("severity", Schema::Value),
    ("exclude_lints", Schema::Value),
    ("ignore", Schema::Value),
    ("lint_on_build", Schema::Value),
    ("lint_specific", Schema::Fields(LINT_SPECIFIC_CONFIG)),
];

const LINT_SPECIFIC_CONFIG: &[(&str, Schema)] =
    &[("mixed_case_exceptions", Schema::Value), ("multi_contract_file_exceptions", Schema::Value)];

const DOC_CONFIG: &[(&str, Schema)] = &[
    ("out", Schema::Value),
    ("title", Schema::Value),
    ("book", Schema::Value),
    ("homepage", Schema::Value),
    ("repository", Schema::Value),
    ("commit", Schema::Value),
    ("path", Schema::Value),
    ("ignore", Schema::Value),
];

const BIND_JSON_CONFIG: &[(&str, Schema)] =
    &[("out", Schema::Value), ("include", Schema::Value), ("exclude", Schema::Value)];

const PATH_PERMISSION: &[(&str, Schema)] = &[("access", Schema::Value), ("path", Schema::Value)];

const VYPER_CONFIG: &[(&str, Schema)] = &[
    ("optimize", Schema::Value),
    ("opt_level", Schema::Value),
    ("optLevel", Schema::Value),
    ("path", Schema::Value),
    ("experimental_codegen", Schema::Value),
    ("venom_experimental", Schema::Value),
    ("debug", Schema::Value),
    ("enable_decimals", Schema::Value),
    ("venom", Schema::Fields(VYPER_VENOM_SETTINGS)),
];

const VYPER_VENOM_SETTINGS: &[(&str, Schema)] = &[
    ("disableInlining", Schema::Value),
    ("disable_inlining", Schema::Value),
    ("disableCSE", Schema::Value),
    ("disable_cse", Schema::Value),
    ("disableSCCP", Schema::Value),
    ("disable_sccp", Schema::Value),
    ("disableLoadElimination", Schema::Value),
    ("disable_load_elimination", Schema::Value),
    ("disableDeadStoreElimination", Schema::Value),
    ("disable_dead_store_elimination", Schema::Value),
    ("disableAlgebraicOptimization", Schema::Value),
    ("disable_algebraic_optimization", Schema::Value),
    ("disableBranchOptimization", Schema::Value),
    ("disable_branch_optimization", Schema::Value),
    ("disableAssertElimination", Schema::Value),
    ("disable_assert_elimination", Schema::Value),
    ("disableMem2Var", Schema::Value),
    ("disable_mem2var", Schema::Value),
    ("disableSimplifyCFG", Schema::Value),
    ("disable_simplify_cfg", Schema::Value),
    ("disableRemoveUnusedVariables", Schema::Value),
    ("disable_remove_unused_variables", Schema::Value),
    ("inlineThreshold", Schema::Value),
    ("inline_threshold", Schema::Value),
];

const MAP_DEPENDENCY: &[(&str, Schema)] = &[
    ("version", Schema::Value),
    ("url", Schema::Value),
    ("git", Schema::Value),
    ("rev", Schema::Value),
    ("branch", Schema::Value),
    ("tag", Schema::Value),
    ("project_root", Schema::Value),
];

const SOLDEER_CONFIG: &[(&str, Schema)] = &[
    ("remappings_generate", Schema::Value),
    ("remappings_regenerate", Schema::Value),
    ("remappings_version", Schema::Value),
    ("remappings_prefix", Schema::Value),
    ("remappings_location", Schema::Value),
    ("recursive_deps", Schema::Value),
];

const SETTINGS_OVERRIDES: &[(&str, Schema)] = &[
    ("name", Schema::Value),
    ("via_ir", Schema::Value),
    ("evm_version", Schema::Value),
    ("optimizer", Schema::Value),
    ("optimizer_runs", Schema::Value),
    ("bytecode_hash", Schema::Value),
];

const COMPILATION_RESTRICTIONS: &[(&str, Schema)] = &[
    ("paths", Schema::Value),
    ("version", Schema::Value),
    ("via_ir", Schema::Value),
    ("bytecode_hash", Schema::Value),
    ("min_optimizer_runs", Schema::Value),
    ("optimizer_runs", Schema::Value),
    ("max_optimizer_runs", Schema::Value),
    ("min_evm_version", Schema::Value),
    ("evm_version", Schema::Value),
    ("max_evm_version", Schema::Value),
];
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_serialized_defaults_and_dynamic_names() {
        validate_toml_keys(
            &Config::default().to_string_pretty().unwrap(),
            Path::new("foundry.toml"),
        )
        .unwrap();
        validate_toml_keys(
            r#"
            [rpc_endpoints.arbitrary-name]
            url = "${RPC_URL}"
            retries = 3
            [etherscan.custom-chain]
            key = "${EXPLORER_KEY}"
            url = "https://example.com/api"
            [dependencies.custom-package]
            version = "1.0"
            git = "https://example.com/package"
            [profile.default.symbolic.dynamic_lengths]
            "MyContract.items" = [1, 2]
            [external.custom_tool]
            anything = { nested = true }
            "#,
            Path::new("foundry.toml"),
        )
        .unwrap();
        validate_toml_keys(
            r#"
            [rpc_endpoints]
            mainnet = "https://example.com"
            [dependencies]
            forge_std = "1.9"
            [profile.default]
            extends = "base.toml"
            "#,
            Path::new("foundry.toml"),
        )
        .unwrap();
    }

    #[test]
    fn rejects_ignored_optimizer_spellings_and_unknown_legacy_profile_keys() {
        let err = validate_toml_keys(
            r#"
            [profile.ci.optimizer_details]
            yul_details = { optimizer_steps = "u" }
            [custom_tool]
            flag = true
            "#,
            Path::new("base.toml"),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            "Unknown configuration keys in base.toml: custom_tool.flag, profile.ci.optimizer_details.yul_details"
        );
    }

    #[test]
    fn rejects_invalid_container_in_inactive_profile() {
        let err = validate_toml_keys("[[profile.ci.fuzz]]\nrunz = 1\n", Path::new("foundry.toml"))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "Invalid configuration values in foundry.toml: profile.ci.fuzz (expected table)"
        );
    }

    #[test]
    fn network_keys_match_enabled_network_features() {
        let optimism =
            validate_toml_keys("[profile.default]\noptimism = true\n", Path::new("foundry.toml"));
        assert_eq!(optimism.is_ok(), NetworkConfigs::is_config_key("optimism"));

        let monad =
            validate_toml_keys("[profile.default]\nmonad = true\n", Path::new("foundry.toml"));
        assert_eq!(monad.is_ok(), NetworkConfigs::is_config_key("monad"));
    }
}
