#[cfg(test)]
mod tests {
    use crate::test_helpers::{COMPILED, PROJECT};
    use foundry_config::{FuzzConfig, InlineConfig, InvariantConfig};

    #[test]
    fn inline_fuzz_config() {
        let root = &PROJECT.paths.root;
        let compiled = COMPILED.clone();
        let base_fuzz = FuzzConfig::default();
        if let Ok(conf) = InlineConfig::<FuzzConfig>::try_from((&compiled, &base_fuzz, root)) {
            // Inline config defined in testdata/inline/FuzzInlineConf.t.sol
            let contract_name = "inline/FuzzInlineConf.t.sol:FuzzInlineConf";
            let function_name = "testFailFuzz";
            let inline_config: &FuzzConfig = conf.get_config(contract_name, function_name).unwrap();
            assert_eq!(inline_config.runs, 1024);
            assert_eq!(inline_config.max_test_rejects, 500);
        }
    }

    #[test]
    fn inline_invariant_config() {
        let root = &PROJECT.paths.root;
        let compiled = COMPILED.clone();
        let base_invariant = InvariantConfig::default();
        if let Ok(conf) = InlineConfig::<InvariantConfig>::try_from((&compiled, &base_invariant, root)) {
            // Inline config defined in testdata/inline/InvariantInlineConf.t.sol
            let contract_name = "inline/InvariantInlineConf.t.sol:InvariantInlineConf";
            let function_name = "invariant_neverFalse";
            let inline_config: &InvariantConfig =
                conf.get_config(contract_name, function_name).unwrap();
            assert_eq!(inline_config.runs, 333);
            assert_eq!(inline_config.depth, 32);
            assert_eq!(inline_config.fail_on_revert, false);
            assert_eq!(inline_config.call_override, true);
        }
    }
}
