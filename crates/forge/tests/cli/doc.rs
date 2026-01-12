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
