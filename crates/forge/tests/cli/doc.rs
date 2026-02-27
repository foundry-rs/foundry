use foundry_test_utils::util::{RemoteProject, setup_forge_remote};

#[test]
fn can_generate_solmate_docs() {
    let (prj, _) =
        setup_forge_remote(RemoteProject::new("transmissions11/solmate").set_build(false));
    prj.forge_command().args(["doc", "--build"]).assert_success();
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
    /// @notice Process a single address
    /// @param addr The address to process
    function process(address addr) external;

    /// @notice Process multiple addresses
    /// @param addrs The addresses to process
    function process(address[] calldata addrs) external;

    /// @notice Process an address with a value
    /// @param addr The address to process
    /// @param value The value to use
    function process(address addr, uint256 value) external;
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
    function process(address addr) external {
        // Implementation for single address
    }

    /// @inheritdoc IExample
    function process(address[] calldata addrs) external {
        // Implementation for multiple addresses
    }

    /// @inheritdoc IExample
    function process(address addr, uint256 value) external {
        // Implementation for address with value
    }
}
"#,
    );

    cmd.args(["doc", "--build"]).assert_success();

    let doc_path = prj.root().join("docs/src/src/Example.sol/contract.Example.md");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(content.contains("Process a single address"));
    assert!(content.contains("Process multiple addresses"));
    assert!(content.contains("Process an address with a value"));
});

// Test that hyperlinks use relative paths, not absolute paths
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

    cmd.args(["doc", "--build"]).assert_success();

    let doc_path = prj.root().join("docs/src/src/Derived.sol/contract.Derived.md");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    assert!(
        content.contains("[IBase](/src/IBase.sol/interface.IBase.md")
            || content.contains("[IBase](\\src\\IBase.sol\\interface.IBase.md"),
        "Hyperlink should use relative path but found: {:?}",
        content.lines().find(|line| line.contains("[IBase]")).unwrap_or("not found")
    );
});

// Test that constants and immutables are documented under "Constants" section when only constants
// are present fixes <https://github.com/foundry-rs/foundry/issues/4611>
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

    cmd.args(["doc", "--build"]).assert_success();

    let doc_path =
        prj.root().join("docs/src/src/CounterConstants.sol/contract.CounterConstants.md");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    // Check that Constants section exists
    assert!(content.contains("## Constants"), "Should have Constants section");
    // Check that State Variables section does not exist
    assert!(!content.contains("## State Variables"), "Should not have State Variables section");

    // Get the position of the Constants section and of the Functions section
    let constants_section_pos = content.find("## Constants").unwrap();
    let functions_section_pos = content.find("## Functions").unwrap();

    // Check that Constants section contains the constant
    assert!(content.contains("### FOO"), "Should have FOO constant");
    let foo_constant_pos = content.find("## FOO").unwrap();
    assert!(
        foo_constant_pos > constants_section_pos && foo_constant_pos < functions_section_pos,
        "FOO constant should be after Constants section and before Functions section"
    );

    // Check that Constants section contains the immutable
    let bar_immutable_pos = content.find("## BAR").unwrap();
    assert!(content.contains("### BAR"), "Should have BAR immutable");
    assert!(
        bar_immutable_pos > constants_section_pos && bar_immutable_pos < functions_section_pos,
        "BAR immutable should be after Constants section and before Functions section"
    );
});

// Test that state variables are documented under "State Variables" section when only state
// variables are present fixes <https://github.com/foundry-rs/foundry/issues/4611>
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

    cmd.args(["doc", "--build"]).assert_success();

    let doc_path =
        prj.root().join("docs/src/src/CounterStateVariables.sol/contract.CounterStateVariables.md");
    let content = std::fs::read_to_string(&doc_path).unwrap();

    // Check that Constants section does not exist
    assert!(!content.contains("## Constants"), "Should not have Constants section");
    // Check that State Variables section exists
    assert!(content.contains("## State Variables"), "Should have State Variables section");

    // Get the position of the State Variables section and of the Functions section
    let state_variables_section_pos = content.find("## State Variables").unwrap();
    let functions_section_pos = content.find("## Functions").unwrap();

    // Check that State Variables section contains the state variable
    assert!(content.contains("### baz"), "Should have baz state variable");
    let baz_state_variable_pos = content.find("## baz").unwrap();
    assert!(
        baz_state_variable_pos > state_variables_section_pos
            && baz_state_variable_pos < functions_section_pos,
        "baz state variable should be after State Variables section and before Functions section"
    );
});

// Test that constants/immutables and state-variables are documented under separate sections when
// both are present fixes <https://github.com/foundry-rs/foundry/issues/4611>
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

        cmd.args(["doc", "--build"]).assert_success();

        let doc_path = prj
            .root()
            .join("docs/src/src/CounterMixedVariables.sol/contract.CounterMixedVariables.md");
        let content = std::fs::read_to_string(&doc_path).unwrap();

        // Check that Constants section and the State Variables section exist
        assert!(content.contains("## Constants"), "Should have Constants section");
        assert!(content.contains("## State Variables"), "Should have State Variables section");

        // Get the position of the Constants, State Variables, and Functions sections
        let constants_section_pos = content.find("## Constants").unwrap();
        let state_variables_section_pos = content.find("## State Variables").unwrap();
        let functions_section_pos = content.find("## Functions").unwrap();

        // Validate that the sections are in the correct order
        assert!(
            constants_section_pos < state_variables_section_pos
                && state_variables_section_pos < functions_section_pos,
            "Constants section should be before State Variables section and before Functions section"
        );

        // Check that Constants section contains the constant
        assert!(content.contains("### FOO"), "Should have FOO constant");
        let foo_constant_pos = content.find("## FOO").unwrap();
        assert!(
            foo_constant_pos > constants_section_pos
                && foo_constant_pos < state_variables_section_pos,
            "FOO constant should be after Constants section and before State Variables section"
        );

        // Check that Constants section contains the immutable
        assert!(content.contains("### BAR"), "Should have BAR immutable");
        let bar_immutable_pos = content.find("## BAR").unwrap();
        assert!(
            bar_immutable_pos > constants_section_pos
                && bar_immutable_pos < state_variables_section_pos,
            "BAR immutable should be after Constants section and before State Variables section"
        );

        // Check that State Variables section contains the state variable
        assert!(content.contains("### baz"), "Should have baz state variable");
        let baz_state_variable_pos = content.find("## baz").unwrap();
        assert!(
            baz_state_variable_pos > state_variables_section_pos
                && baz_state_variable_pos < functions_section_pos,
            "baz state variable should be after State Variables section and before Functions section"
        );
    }
);
