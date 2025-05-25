use foundry_config::{LintSeverity, LinterConfig};

const CONTRACT: &str = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

struct _PascalCaseInfo { uint256 a; }
uint256 constant screaming_snake_case_info = 0;

contract ContractWithLints {
    uint256 VARIABLE_MIXED_CASE_INFO;

    function incorrectShiftHigh() public {
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
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ..Default::default()
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
warning[divide-before-multiply]: multiplication should occur before division to avoid loss of precision
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |


"#]]);
});

forgetest!(can_use_config_ignore, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContract", OTHER_CONTRACT).unwrap();

    // Check config for `ignore`
    prj.update_config(|config| {
        config.lint =
            LinterConfig { ignore: vec!["src/ContractWithLints.sol".into()], ..Default::default() };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:6:17
  |
6 |         uint256 VARIABLE_MIXED_CASE_INFO;
  |                 ------------------------
  |


"#]]);

    // Check config again, ignoring all files
    prj.update_config(|config| {
        config.lint = LinterConfig {
            ignore: vec!["src/ContractWithLints.sol".into(), "src/OtherContract.sol".into()],
            ..Default::default()
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[""]]);
});

forgetest!(can_override_config_severity, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Override severity
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            ignore: vec!["src/ContractWithLints.sol".into()],
            ..Default::default()
        };
    });
    cmd.arg("lint").args(["--severity", "info"]).assert_success().stderr_eq(str![[r#"
note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:6:17
  |
6 |         uint256 VARIABLE_MIXED_CASE_INFO;
  |                 ------------------------
  |


"#]]);
});

forgetest!(can_override_config_path, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Override excluded files
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec!["src/ContractWithLints.sol".into()],
        };
    });
    cmd.arg("lint").arg("src/ContractWithLints.sol").assert_success().stderr_eq(str![[r#"
warning[divide-before-multiply]: multiplication should occur before division to avoid loss of precision
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |


"#]]);
});

forgetest!(can_override_config_lint, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();

    // Override excluded lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ..Default::default()
        };
    });
    cmd.arg("lint").args(["--only-lint", "incorrect-shift"]).assert_success().stderr_eq(str![[
        r#"
warning[incorrect-shift]: the order of args in a shift operation is incorrect
  [FILE]:13:18
   |
13 |         result = 8 >> localValue;
   |                  ---------------
   |


"#
    ]]);
});
