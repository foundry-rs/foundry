use foundry_test_utils::util::{RemoteProject, setup_forge_remote};

#[test]
fn can_generate_solmate_docs() {
    let (prj, _) =
        setup_forge_remote(RemoteProject::new("transmissions11/solmate").set_build(false));
    prj.forge_command().args(["doc"]).assert_success();
    // At least one MDX page was generated.
    assert!(
        std::fs::read_dir(prj.root().join("docs/src/pages/src")).is_ok(),
        "docs/src/pages/src directory should exist"
    );
}

// Test that overloaded functions in interfaces inherit the correct NatSpec comments
// fixes <https://github.com/foundry-rs/foundry/issues/11823>
forgetest_init!(can_generate_docs_for_overloaded_functions, |prj, cmd| {
    prj.add_source(
        "IExample.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IExample {
    /// @notice Deposit tokens into the vault
    /// @param amount The amount to deposit
    function deposit(uint256 amount) external;

    /// @notice Withdraw tokens from the vault
    /// @param amount The amount to withdraw
    function withdraw(uint256 amount) external;
}
"#,
    );

    prj.add_source(
        "Example.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IExample.sol";

contract Example is IExample {
    /// @inheritdoc IExample
    function deposit(uint256 amount) external {}

    /// @inheritdoc IExample
    function withdraw(uint256 amount) external {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Example.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("Deposit tokens into the vault"),
        "deposit notice should be inherited"
    );
    assert!(
        content.contains("Withdraw tokens from the vault"),
        "withdraw notice should be inherited"
    );
});

// Test that {Ident} cross-references resolve to root-relative vocs links.
// fixes <https://github.com/foundry-rs/foundry/issues/12361>
forgetest_init!(hyperlinks_use_relative_paths, |prj, cmd| {
    prj.add_source(
        "IBase.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IBase {
    function baseFunction() external;
}
"#,
    );

    prj.add_source(
        "Derived.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IBase.sol";

/// @dev Inherits: {IBase}
contract Derived is IBase {
    function baseFunction() external override {}
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.Derived.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("[IBase](/src/interface.IBase)"),
        "Hyperlink should be a root-relative vocs link, found: {:?}",
        content.lines().find(|line| line.contains("[IBase]")).unwrap_or("not found")
    );
});

// Test that constants and immutables are documented under "Constants" section when only constants
// are present.
// fixes <https://github.com/foundry-rs/foundry/issues/4611>
forgetest_init!(constants_and_immutables_are_documented_under_constants_section, |prj, cmd| {
    prj.add_source(
        "CounterConstants.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19;

contract CounterConstants {
    uint256 public constant FOO = 1;
    uint256 public immutable BAR;

    constructor() {
        BAR = 2;
    }
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.CounterConstants.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(content.contains("## Constants"), "Should have Constants section");
    assert!(!content.contains("## State Variables"), "Should not have State Variables section");

    let constants_pos = content.find("## Constants").unwrap();
    let functions_pos = content.find("## Functions").unwrap();

    assert!(content.contains("### FOO"), "Should have FOO constant");
    let foo_pos = content.find("### FOO").unwrap();
    assert!(foo_pos > constants_pos && foo_pos < functions_pos, "FOO should be inside Constants");

    assert!(content.contains("### BAR"), "Should have BAR immutable");
    let bar_pos = content.find("### BAR").unwrap();
    assert!(bar_pos > constants_pos && bar_pos < functions_pos, "BAR should be inside Constants");
});

// Test that state variables are documented under "State Variables" section when only state
// variables are present.
// fixes <https://github.com/foundry-rs/foundry/issues/4611>
forgetest_init!(state_variables_are_documented_under_state_variables_section, |prj, cmd| {
    prj.add_source(
        "CounterStateVariables.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19;

contract CounterStateVariables {
    uint256 public baz;

    function increment() public {
        baz++;
    }
}
"#,
    );

    cmd.args(["doc"]).assert_success();

    let doc_path = prj.root().join("docs/src/pages/src/contract.CounterStateVariables.mdx");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(!content.contains("## Constants"), "Should not have Constants section");
    assert!(content.contains("## State Variables"), "Should have State Variables section");

    let state_vars_pos = content.find("## State Variables").unwrap();
    let functions_pos = content.find("## Functions").unwrap();

    assert!(content.contains("### baz"), "Should have baz state variable");
    let baz_pos = content.find("### baz").unwrap();
    assert!(
        baz_pos > state_vars_pos && baz_pos < functions_pos,
        "baz should be inside State Variables"
    );
});

// Test that constants/immutables and state-variables are documented under separate sections when
// both are present.
// fixes <https://github.com/foundry-rs/foundry/issues/4611>
forgetest_init!(
    constants_and_immutables_and_state_variables_are_documented_under_separate_sections,
    |prj, cmd| {
        prj.add_source(
            "CounterMixedVariables.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19;

contract CounterMixedVariables {
    uint256 public constant FOO = 1;
    uint256 public immutable BAR;
    uint256 public baz;

    constructor() {
        BAR = 2;
    }

    function increment() public {
        baz++;
    }
}
"#,
        );

        cmd.args(["doc"]).assert_success();

        let doc_path = prj.root().join("docs/src/pages/src/contract.CounterMixedVariables.mdx");
        let content = std::fs::read_to_string(&doc_path).unwrap();

        assert!(content.contains("## Constants"), "Should have Constants section");
        assert!(content.contains("## State Variables"), "Should have State Variables section");

        let constants_pos = content.find("## Constants").unwrap();
        let state_vars_pos = content.find("## State Variables").unwrap();
        let functions_pos = content.find("## Functions").unwrap();

        assert!(
            constants_pos < state_vars_pos && state_vars_pos < functions_pos,
            "Constants < State Variables < Functions"
        );

        assert!(content.contains("### FOO"), "Should have FOO constant");
        let foo_pos = content.find("### FOO").unwrap();
        assert!(
            foo_pos > constants_pos && foo_pos < state_vars_pos,
            "FOO should be inside Constants"
        );

        assert!(content.contains("### BAR"), "Should have BAR immutable");
        let bar_pos = content.find("### BAR").unwrap();
        assert!(
            bar_pos > constants_pos && bar_pos < state_vars_pos,
            "BAR should be inside Constants"
        );

        assert!(content.contains("### baz"), "Should have baz state variable");
        let baz_pos = content.find("### baz").unwrap();
        assert!(
            baz_pos > state_vars_pos && baz_pos < functions_pos,
            "baz should be inside State Variables"
        );
    }
);
