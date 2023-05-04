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
    use foundry_config::{FuzzConfig, InvariantConfig};

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
    fn build_test_options() {
        let root = &PROJECT.paths.root;
        let profiles = vec!["default".to_string(), "ci".to_string()];
        let build_result = TestOptionsBuilder::default()
            .fuzz(FuzzConfig::default())
            .invariant(InvariantConfig::default())
            .compile_output(&COMPILED)
            .profiles(profiles)
            .build(root);

        assert!(build_result.is_ok());
    }

    #[test]
    fn build_test_options_invalid_profile() {
        let root = &PROJECT.paths.root;
        let profiles = vec!["profile-sheldon-cooper".to_string()];
        let build_result = TestOptionsBuilder::default()
            .fuzz(FuzzConfig::default())
            .invariant(InvariantConfig::default())
            .compile_output(&COMPILED)
            .profiles(profiles)
            .build(root);

        assert!(build_result.is_err());
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
