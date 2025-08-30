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
        function functionMIXEDCaseInfo() public {}
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

const COUNTER_A: &str = r#"
    // SPDX-License-Identifier: MIT
    pragma solidity ^0.8.0;

    contract CounterA {
        uint256 public CounterA_Fail_Lint;
    }
        "#;

const COUNTER_B: &str = r#"
    // SPDX-License-Identifier: MIT
    pragma solidity ^0.8.0;

    contract CounterB {
        uint256 public CounterB_Fail_Lint;
    }
        "#;

const COUNTER_WITH_CONST: &str = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

uint256 constant MAX = 1000000;

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
        "#;

const COUNTER_TEST_WITH_CONST: &str = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import { Counter, MAX } from "../src/Counter.sol";

contract CounterTest {
  Counter public counter;

  function setUp() public {
    counter = new Counter();
  }

  function testFuzz_setNumber(uint256[MAX] calldata numbers) public {
    for (uint256 i = 0; i < numbers.length; ++i) {
      counter.setNumber(numbers[i]);
    }
  }
}
        "#;

forgetest!(can_use_config, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT);

    // Check config for `severity` and `exclude`
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
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
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#divide-before-multiply


"#]]);
});

forgetest!(can_use_config_ignore, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContract", OTHER_CONTRACT);

    // Check config for `ignore`
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![],
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into()],
            lint_on_build: true,
            ..Default::default()
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[r#"
note[mixed-case-function]: function names should use mixedCase
 [FILE]:9:18
  |
9 |         function functionMIXEDCaseInfo() public {}
  |                  ---------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-function


"#]]);

    // Check config again, ignoring all files
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![],
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into(), "src/OtherContract.sol".into()],
            lint_on_build: true,
            ..Default::default()
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[""]]);
});

forgetest!(can_use_config_mixed_case_exception, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContract", OTHER_CONTRACT);

    // Check config for `ignore`
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![],
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into()],
            lint_on_build: true,
            mixed_case_exceptions: vec!["MIXED".to_string()],
        };
    });
    cmd.arg("lint").assert_success().stderr_eq(str![[""]]);
});

forgetest!(can_override_config_severity, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT);

    // Override severity
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec![],
            ignore: vec!["src/ContractWithLints.sol".into()],
            lint_on_build: true,
            ..Default::default()
        };
    });
    cmd.arg("lint").args(["--severity", "info"]).assert_success().stderr_eq(str![[r#"
note[mixed-case-function]: function names should use mixedCase
 [FILE]:9:18
  |
9 |         function functionMIXEDCaseInfo() public {}
  |                  ---------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-function


"#]]);
});

forgetest!(can_override_config_path, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT);

    // Override excluded files
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec!["src/ContractWithLints.sol".into()],
            lint_on_build: true,
            ..Default::default()
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
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT);

    // Override excluded lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::High, LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
            ..Default::default()
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
    prj.add_source("ContractWithLints", CONTRACT);

    // Configure linter to show only medium severity lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
            ..Default::default()
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
    prj.add_source("ContractWithLints", CONTRACT);

    // Configure linter to show medium severity lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
            ..Default::default()
        };
    });

    // Run forge build with --quiet flag - should not show linting output
    cmd.arg("build").arg("--quiet").assert_success().stderr_eq(str![[""]]).stdout_eq(str![[""]]);
});

forgetest!(build_with_json_uses_json_linter_output, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);

    // Configure linter to show medium severity lints
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: true,
            ..Default::default()
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
    prj.add_source("ContractWithLints", CONTRACT);

    // Configure linter with medium severity lints but disable lint_on_build
    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::Med],
            exclude_lints: vec!["incorrect-shift".into()],
            ignore: vec![],
            lint_on_build: false,
            ..Default::default()
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
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT);
    cmd.arg("lint").assert_success();

    prj.wipe_contracts();
    prj.add_source("OtherContractWithLints", OTHER_CONTRACT);
    prj.add_source("ContractWithLints", CONTRACT);
    cmd.arg("lint").assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/11080>
forgetest!(can_use_only_lint_with_multilint_passes, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("ContractWithLints", CONTRACT);
    prj.add_source("OnlyImports", ONLY_IMPORTS);
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

// <https://github.com/foundry-rs/foundry/issues/11234>
forgetest!(can_lint_only_built_files, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("CounterAWithLints", COUNTER_A);
    prj.add_source("CounterBWithLints", COUNTER_B);

    // Both contracts should be linted on build. Redact contract as order is not guaranteed.
    cmd.forge_fuse().args(["build"]).assert_success().stderr_eq(str![[r#"
note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:6:24
  |
6 |         uint256 public Counter[..]_Fail_Lint;
  |                        ------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-variable

note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:6:24
  |
6 |         uint256 public Counter[..]_Fail_Lint;
  |                        ------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-variable


"#]]);
    // Only contract CounterBWithLints that we build should be linted.
    cmd.forge_fuse().args(["build", "src/CounterBWithLints.sol"]).assert_success().stderr_eq(str![
        [r#"
note[mixed-case-variable]: mutable variables should use mixedCase
 [FILE]:6:24
  |
6 |         uint256 public CounterB_Fail_Lint;
  |                        ------------------
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#mixed-case-variable


"#]
    ]);
});

// <https://github.com/foundry-rs/foundry/issues/11392>
forgetest!(can_lint_param_constants, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("Counter", COUNTER_WITH_CONST);
    prj.add_test("CounterTest", COUNTER_TEST_WITH_CONST);

    cmd.forge_fuse().args(["build"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/11460>
forgetest!(lint_json_output_no_ansi_escape_codes, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source(
        "UnwrappedModifierTest",
        r#"
        // SPDX-License-Identifier: MIT
        pragma solidity ^0.8.0;

        contract UnwrappedModifierTest {
            mapping(address => bool) isOwner;

            modifier onlyOwner() {
                require(isOwner[msg.sender], "Not owner");
                require(msg.sender != address(0), "Zero address");
                _;
            }

            function doSomething() public onlyOwner {}
        }
            "#,
    );

    prj.update_config(|config| {
        config.lint = LinterConfig {
            severity: vec![LintSeverity::CodeSize],
            exclude_lints: vec![],
            ignore: vec![],
            lint_on_build: true,
            ..Default::default()
        };
    });

    // should produce clean JSON without ANSI escape sequences (for the url nor the snippets)
    cmd.arg("lint").arg("--json").assert_json_stderr(true,
        str![[r#"
            {
              "$message_type": "diag",
              "message": "wrap modifier logic to reduce code size",
              "code": {
                "code": "unwrapped-modifier-logic",
                "explanation": null
              },
              "level": "note",
              "spans": [
                {
                  "file_name": "[..]",
                  "byte_start": 183,
                  "byte_end": 192,
                  "line_start": 8,
                  "line_end": 8,
                  "column_start": 22,
                  "column_end": 31,
                  "is_primary": true,
                  "text": [
                    {
                      "text": "            modifier onlyOwner() {",
                      "highlight_start": 22,
                      "highlight_end": 31
                    }
                  ],
                  "label": null
                }
              ],
              "children": [
                {
                  "message": "wrap modifier logic to reduce code size\n\n- modifier onlyOwner() {\n-     require(isOwner[msg.sender], \"Not owner\");\n-     require(msg.sender != address(0), \"Zero address\");\n-     _;\n- }\n+ modifier onlyOwner() {\n+     _onlyOwner();\n+     _;\n+ }\n+ \n+ function _onlyOwner() internal {\n+     require(isOwner[msg.sender], \"Not owner\");\n+     require(msg.sender != address(0), \"Zero address\");\n+ }\n\n",
                  "code": null,
                  "level": "note",
                  "spans": [],
                  "children": [],
                  "rendered": null
                },
                {
                  "message": "https://book.getfoundry.sh/reference/forge/forge-lint#unwrapped-modifier-logic",
                  "code": null,
                  "level": "help",
                  "spans": [],
                  "children": [],
                  "rendered": null
                }
              ],
              "rendered": "note[unwrapped-modifier-logic]: wrap modifier logic to reduce code size\n  |\n8 |             modifier onlyOwner() {\n  |                      ---------\n  |\n  = note: wrap modifier logic to reduce code size\n          \n          - modifier onlyOwner() {\n          -     require(isOwner[msg.sender], \"Not owner\");\n          -     require(msg.sender != address(0), \"Zero address\");\n          -     _;\n          - }\n          + modifier onlyOwner() {\n          +     _onlyOwner();\n          +     _;\n          + }\n          + \n          + function _onlyOwner() internal {\n          +     require(isOwner[msg.sender], \"Not owner\");\n          +     require(msg.sender != address(0), \"Zero address\");\n          + }\n          \n  = help: https://book.getfoundry.sh/reference/forge/forge-lint#unwrapped-modifier-logic\n\n --> [..]\n"
            }
"#]],
);
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
