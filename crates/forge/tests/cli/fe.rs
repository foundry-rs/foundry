//! Native Fe compilation and test execution from Fe and Solidity.

use std::{fs, path::PathBuf};

forgetest!(
    #[ignore = "requires FE_TEST_BIN (Fe 26.3+) and FE_TEST_SOLC (solc 0.8.30+)"]
    native_fe_build_test_and_dependency_cache,
    |prj, cmd| {
        let compiler = PathBuf::from(std::env::var_os("FE_TEST_BIN").expect("set FE_TEST_BIN"));
        let solc = PathBuf::from(std::env::var_os("FE_TEST_SOLC").expect("set FE_TEST_SOLC"));
        for (path, source) in [
            ("foundry.toml", include_str!("../../../../testdata/fe/foundry.toml")),
            ("src/counter/fe.toml", include_str!("../../../../testdata/fe/src/counter/fe.toml")),
            (
                "src/counter/src/lib.fe",
                include_str!("../../../../testdata/fe/src/counter/src/lib.fe"),
            ),
            ("lib/step/fe.toml", include_str!("../../../../testdata/fe/lib/step/fe.toml")),
            ("lib/step/src/lib.fe", include_str!("../../../../testdata/fe/lib/step/src/lib.fe")),
            ("test/Counter.t.sol", include_str!("../../../../testdata/fe/test/Counter.t.sol")),
            ("test/NativeTest.fe", include_str!("../../../../testdata/fe/test/NativeTest.fe")),
        ] {
            let path = prj.root().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        prj.update_config(|config| config.fe.path = Some(compiler));
        cmd.args(["test", "-q", "--use"]).arg(&solc).assert_success().stderr_eq("");
        cmd.forge_fuse().args(["build", "--use"]).arg(&solc).assert_success().stdout_eq(str![[
            r#"
No files changed, compilation skipped

"#
        ]]);
        fs::write(prj.root().join("lib/step/src/lib.fe"), "pub fn increment_by() -> u256 { 2 }\n")
            .unwrap();
        cmd.forge_fuse()
            .args(["test", "-q", "--match-test", "testIncrementUsesFeDependency", "--use"])
            .arg(&solc)
            .assert_failure();
    }
);

// A standalone Fe test contract uses the same ABI conventions as Solidity and Vyper.
forgetest!(
    #[ignore = "requires FE_TEST_BIN (Fe 26.3+) and FE_TEST_SOLC (solc 0.8.30+)"]
    native_fe_authored_tests,
    |prj, cmd| {
        let compiler = PathBuf::from(std::env::var_os("FE_TEST_BIN").expect("set FE_TEST_BIN"));
        let solc = PathBuf::from(std::env::var_os("FE_TEST_SOLC").expect("set FE_TEST_SOLC"));
        let source = include_str!("../../../../testdata/fe/test/NativeTest.fe");
        let test = prj.root().join("test/NativeTest.fe");
        fs::create_dir_all(test.parent().unwrap()).unwrap();
        fs::write(&test, source).unwrap();
        prj.update_config(|config| {
            config.fe.path = Some(compiler);
            config.evm_version = foundry_compilers::artifacts::EvmVersion::Osaka;
            config.fuzz.runs = 256;
        });
        cmd.args(["test", "--use"]).arg(&solc).assert_success().stdout_eq(str![[r#"
...
Ran 5 tests for test/NativeTest.fe:NativeTest
[PASS] testDeal() ([GAS])
[PASS] testDeployment() ([GAS])
[PASS] testFuzzSetNumber(uint256) (runs: 256, [..])
[PASS] testPrank() ([GAS])
[PASS] testSetup() ([GAS])
Suite result: ok. 5 passed; 0 failed; 0 skipped; [..]

Ran 1 test suite [..]: 5 tests passed, 0 failed, 0 skipped (5 total tests)

"#]]);

        // Verify that Forge executes assertions, rather than merely discovering tests.
        fs::write(&test, source.replace("assert(number == 42)", "assert(number == 0)")).unwrap();
        cmd.forge_fuse()
            .args(["test", "--match-test", "testSetup", "--use"])
            .arg(&solc)
            .assert_failure()
            .stdout_eq(str![[r#"
...
[FAIL: panic: assertion failed (0x01)] testSetup() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [..]
...
"#]]);
    }
);
