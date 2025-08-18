use forge_lint::{linter::Lint, sol::med::REGISTERED_LINTS};
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
        uint256 result = 8 >> localValue;
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

    // forge-lint: disable-next-line
    import { ContractWithLints } from "./ContractWithLints.sol";

    contract OtherContractWithLints {
        uint256 VARIABLE_MIXED_CASE_INFO;
    }
        "#;

const ONLY_IMPORTS: &str = r#"
    // SPDX-License-Identifier: MIT
    pragma solidity ^0.8.0;

    // forge-lint: disable-next-line
    import { ContractWithLints } from "./ContractWithLints.sol";

    import { _PascalCaseInfo } from "./ContractWithLints.sol";
    import "./ContractWithLints.sol";
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
            ignore: vec![],
            lint_on_build: true,
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
warning[divide-before-multiply]: multiplication should occur before division to avoid loss of precision
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#divide-before-multiply


"#]]);
});

forgetest!(can_use_config_ignore, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContract", OTHER_CONTRACT).unwrap();

    // Check config for `ignore`
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![],
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into()],
            lint_on_build: true,
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:9:17
  |
9 |         uint256 VARIABLE_MIXED_CASE_INFO;
  |                 ------------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-variable


"#]]);

    // Check config again, ignoring all files
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![],
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into(), "src/OtherContract.sol".into()],
            lint_on_build: true,
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
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into()],
            lint_on_build: true,
        };
    });
    cmd.arg("lint").args(["--severity", "info"]).assert_success().stderr_eq(str![[r#"
note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:9:17
  |
9 |         uint256 VARIABLE_MIXED_CASE_INFO;
  |                 ------------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-variable


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
            lint_on_build: true,
        };
    });
    cmd.arg("lint").arg("src/ContractWithLints.sol").assert_success().stderr_eq(str![[r#"
warning[divide-before-multiply]: multiplication should occur before division to avoid loss of precision
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#divide-before-multiply


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
            ignore: vec![],
            lint_on_build: true,
        };
    });
    cmd.arg("lint").args(["--only-lint", "incorrect-shift"]).assert_success().stderr_eq(str![[
        r#"
warning[incorrect-shift]: the order of args in a shift operation is incorrect
  [FILE]:13:26
   |
13 |         uint256 result = 8 >> localValue;
   |                          ---------------
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#incorrect-shift


"#
    ]]);
});

forgetest!(build_runs_linter_by_default, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();

    // Configure linter to show only medium severity lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
        };
    });

    // Run forge build and expect linting output before compilation
    cmd.arg("build").assert_success().stderr_eq(str![[r#"
warning[divide-before-multiply]: multiplication should occur before division to avoid loss of precision
  [FILE]:16:9
   |
16 |         (1 / 2) * 3;
   |         -----------
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#divide-before-multiply


"#]]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2072): Unused local variable.
  [FILE]:13:9:
   |
13 |         uint256 result = 8 >> localValue;
   |         ^^^^^^^^^^^^^^

Warning (6133): Statement has no effect.
  [FILE]:16:9:
   |
16 |         (1 / 2) * 3;
   |         ^^^^^^^^^^^

Warning (2018): Function state mutability can be restricted to pure
  [FILE]:11:5:
   |
11 |     function incorrectShiftHigh() public {
   |     ^ (Relevant source part starts here and spans across multiple lines).

Warning (2018): Function state mutability can be restricted to pure
  [FILE]:15:5:
   |
15 |     function divideBeforeMultiplyMedium() public {
   |     ^ (Relevant source part starts here and spans across multiple lines).

Warning (2018): Function state mutability can be restricted to pure
  [FILE]:18:5:
   |
18 |     function unoptimizedHashGas(uint256 a, uint256 b) public view {
   |     ^ (Relevant source part starts here and spans across multiple lines).


"#]]);
});

forgetest!(build_respects_quiet_flag_for_linting, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();

    // Configure linter to show medium severity lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
        };
    });

    // Run forge build with --quiet flag - should not show linting output
    cmd.arg("build").arg("--quiet").assert_success().stderr_eq(str![[""]]).stdout_eq(str![[""]]);
});

forgetest!(build_with_json_uses_json_linter_output, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();

    // Configure linter to show medium severity lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
        };
    });

    // Run forge build with --json flag - should use JSON formatter for linting
    let output = cmd.arg("build").arg("--json").assert_success();

    // Should contain JSON linting output
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(stderr.contains("\"code\""));
    assert!(stderr.contains("divide-before-multiply"));

    // Should also contain JSON compilation output
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("\"errors\""));
    assert!(stdout.contains("\"sources\""));
});

forgetest!(build_respects_lint_on_build_false, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();

    // Configure linter with medium severity lints but disable lint_on_build
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: false,
        };
    });

    // Run forge build - should NOT show linting output because lint_on_build is false
    cmd.arg("build").assert_success().stderr_eq(str![[""]]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2072): Unused local variable.
  [FILE]:13:9:
   |
13 |         uint256 result = 8 >> localValue;
   |         ^^^^^^^^^^^^^^

Warning (6133): Statement has no effect.
  [FILE]:16:9:
   |
16 |         (1 / 2) * 3;
   |         ^^^^^^^^^^^

Warning (2018): Function state mutability can be restricted to pure
  [FILE]:11:5:
   |
11 |     function incorrectShiftHigh() public {
   |     ^ (Relevant source part starts here and spans across multiple lines).

Warning (2018): Function state mutability can be restricted to pure
  [FILE]:15:5:
   |
15 |     function divideBeforeMultiplyMedium() public {
   |     ^ (Relevant source part starts here and spans across multiple lines).

Warning (2018): Function state mutability can be restricted to pure
  [FILE]:18:5:
   |
18 |     function unoptimizedHashGas(uint256 a, uint256 b) public view {
   |     ^ (Relevant source part starts here and spans across multiple lines).


"#]]);
});

forgetest!(can_process_inline_config_regardless_of_input_order, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();
    cmd.arg("lint").assert_success();

    prj.wipe_contracts();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT).unwrap();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    cmd.arg("lint").assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/11080>
forgetest!(can_use_only_lint_with_multilint_passes, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT).unwrap();
    prj.add_source("OnlyImports", ONLY_IMPORTS).unwrap();
    cmd.arg("lint").args(["--only-lint", "unused-import"]).assert_success().stderr_eq(str![[r#"
note[unused-import]: unused imports should be removed
 [FILE]:8:14
  |
8 |     import { _PascalCaseInfo } from "./ContractWithLints.sol";
  |              ---------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#unused-import


"#]]);
});

// ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn ensure_lint_rule_docs() {
    const FOUNDRY_BOOK_LINT_PAGE_URL: &str =
        "https://book.getfoundry.sh/reference/forge/forge-lint";

    // Fetch the content of the lint reference
    let content = match reqwest::get(FOUNDRY_BOOK_LINT_PAGE_URL).await {
        Ok(resp) => {
            if !resp.status().is_success() {
                panic!(
                    "Failed to fetch Foundry Book lint page ({FOUNDRY_BOOK_LINT_PAGE_URL}). Status: {status}",
                    status = resp.status()
                );
            }
            match resp.text().await {
                Ok(text) => text,
                Err(e) => {
                    panic!("Failed to read response text: {e}");
                }
            }
        }
        Err(e) => {
            panic!("Failed to fetch Foundry Book lint page ({FOUNDRY_BOOK_LINT_PAGE_URL}): {e}",);
        }
    };

    // Ensure no missing lints
    let mut missing_lints = Vec::new();
    for lint in REGISTERED_LINTS {
        let selector = format!("#{}", lint.id());
        if !content.contains(&selector) {
            missing_lints.push(lint.id());
        }
    }

    if !missing_lints.is_empty() {
        let mut msg = String::from(
            "Foundry Book lint validation failed. The following lints must be added to the docs:\n",
        );
        for lint in missing_lints {
            msg.push_str(&format!("  - {lint}\n"));
        }
        msg.push_str("Please open a PR: https://github.com/foundry-rs/book");
        panic!("{msg}");
    }
}

#[test]
fn ensure_no_privileged_lint_id() {
    for lint in REGISTERED_LINTS {
        assert_ne!(lint.id(), "all", "lint-id 'all' is reserved. Please use a different id");
    }
}
