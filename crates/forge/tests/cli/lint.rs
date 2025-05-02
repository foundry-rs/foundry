use foundry_config::{Config, LintSeverity, LinterConfig};

const CONTRACT: &str = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

struct _PascalCaseInfo { uint256 a; }
uint256 constant screaming_snake_case_info = 0;

contract ContractWithLints {
    uint256 VARIABLE_MIXED_CASE_INFO;

    function incorrectShitHigh() public {
        uint256 localValue = 50;
        result = 8 >> localValue;
    }
    function divideBeforeMultiplyMedium() public {
        (1 / 2) * 3;
    }
    function unoptimizedHashGas(uint256 a, uint256 b) public view {
        keccak256(abi.encodePacked(a, b));
    }
    function FUNCTION_MIXED_CASE_INFO() public {}
}
    "#;

const OTHER_CONTRACT: &str = r#"
    // SPDX-License-Identifier: MIT
    pragma solidity ^0.8.0;

    contract ContractWithLints {
        uint256 VARIABLE_MIXED_CASE_INFO;
    }
        "#;

forgetest!(can_use_config, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Check config for `severity` and `exclude`
    prj.write_config(Config {
        lint: LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
warning: divide-before-multiply
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |
   = help: Multiplication should occur before division to avoid loss of precision


"#]]);
});

forgetest!(can_use_config_ignore, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContract", OTHER_CONTRACT).unwrap();

    // Check config for `ignore`
    prj.write_config(Config {
        lint: LinterConfig {
            ignore: vec!["src/ContractWithLints.sol".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
note: variable-mixed-case
 [FILE]:6:9
  |
6 |         uint256 VARIABLE_MIXED_CASE_INFO;
  |         ---------------------------------
  |
  = help: Mutable variables should use mixedCase


"#]]);
});

forgetest!(can_override_config_severity, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Check config for `severity` and `exclude`
    prj.write_config(Config {
        lint: LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            ignore: vec!["src/ContractWithLints.sol".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    cmd.arg("lint").args(["--severity", "info"]).assert_success().stderr_eq(str![[r#"
note: variable-mixed-case
 [FILE]:6:9
  |
6 |         uint256 VARIABLE_MIXED_CASE_INFO;
  |         ---------------------------------
  |
  = help: Mutable variables should use mixedCase


"#]]);
});

forgetest!(can_override_config_path, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Check config for `severity` and `exclude`
    prj.write_config(Config {
        lint: LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec!["src/ContractWithLints.sol".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    cmd.arg("lint").arg("src/ContractWithLints.sol").assert_success().stderr_eq(str![[r#"
warning: divide-before-multiply
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |
   = help: Multiplication should occur before division to avoid loss of precision


"#]]);
});

forgetest!(can_override_config_lint, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Check config for `severity` and `exclude`
    prj.write_config(Config {
        lint: LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    cmd.arg("lint").args(["--only-lint", "incorrect-shift"]).assert_success().stderr_eq(str![[
        r#"
warning: incorrect-shift
  [FILE]:13:18
   |
13 |         result = 8 >> localValue;
   |                  ---------------
   |
   = help: The order of args in a shift operation is incorrect


"#
    ]]);
});
