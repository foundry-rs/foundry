#[cfg(test)]
mod tests {
    use crate::{
        config::runner,
        test_helpers::{filter::Filter, COMPILED, PROJECT},
    };
    use forge::{
        result::{SuiteResult, TestKind, TestResult},
        TestOptions, TestOptionsBuilder,
    };
    use foundry_config::{FuzzConfig, InlineConfig, InvariantConfig};

    #[test]
    fn inline_config_run_fuzz() {
        let opts = test_options();

        let filter = Filter::new(".*", ".*", ".*inline/FuzzInlineConf.t.sol");

        let mut runner = runner();
        runner.test_options = opts.clone();

        let result = runner.test(&filter, None, opts).expect("Test ran");
        let suite_result: &SuiteResult =
            result.get("inline/FuzzInlineConf.t.sol:FuzzInlineConf").unwrap();
        let test_result: &TestResult =
            suite_result.test_results.get("testInlineConfFuzz(uint8)").unwrap();
        match &test_result.kind {
            TestKind::Fuzz { runs, .. } => {
                assert_eq!(runs, &1024);
            }
            _ => {
                assert!(false); // Force test to fail
            }
        }
    }

    #[test]
    fn inline_config_run_invariant() {
        const ROOT: &str = "inline/InvariantInlineConf.t.sol";

        let opts = test_options();
        let filter = Filter::new(".*", ".*", ".*inline/InvariantInlineConf.t.sol");
        let mut runner = runner();
        runner.test_options = opts.clone();

        let result = runner.test(&filter, None, opts).expect("Test ran");

        let suite_result_1 =
            result.get(&format!("{ROOT}:InvariantInlineConf")).expect("Result exists");
        let suite_result_2 =
            result.get(&format!("{ROOT}:InvariantInlineConf2")).expect("Result exists");

        let test_result_1 = suite_result_1.test_results.get("invariant_neverFalse()").unwrap();
        let test_result_2 = suite_result_2.test_results.get("invariant_neverFalse()").unwrap();

        match &test_result_1.kind {
            TestKind::Invariant { runs, .. } => {
                assert_eq!(runs, &333);
            }
            _ => {
                assert!(false); // Force test to fail
            }
        }

        match &test_result_2.kind {
            TestKind::Invariant { runs, .. } => {
                assert_eq!(runs, &42);
            }
            _ => {
                assert!(false); // Force test to fail
            }
        }
    }
    #[test]
    fn inline_fuzz_config() {
        let root = &PROJECT.paths.root;
        let compiled = COMPILED.clone();
        let base_fuzz = FuzzConfig::default();
        if let Ok(conf) = InlineConfig::<FuzzConfig>::try_from((&compiled, &base_fuzz, root)) {
            // Inline config defined in testdata/inline/FuzzInlineConf.t.sol
            let contract_name = "inline/FuzzInlineConf.t.sol:FuzzInlineConf";
            let function_name = "testInlineConfFuzz";
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
        if let Ok(conf) =
            InlineConfig::<InvariantConfig>::try_from((&compiled, &base_invariant, root))
        {
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

    fn test_options() -> TestOptions {
        let root = &PROJECT.paths.root;
        TestOptionsBuilder::default()
            .fuzz(FuzzConfig::default())
            .invariant(InvariantConfig::default())
            .compile_output(&COMPILED)
            .build(root)
            .expect("Config loaded")
    }
}
