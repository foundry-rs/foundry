//! Tests for `forge flatten`.

use foundry_test_utils::{forgetest, util::OutputExt};

// Test that `forge flatten` works on a simple contract with no imports.
forgetest!(flatten_simple_contract, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
"#,
    );

    let output =
        cmd.args(["flatten", "src/Counter.sol"]).assert_success().get_output().stdout_lossy();

    // Should contain combined pragma
    assert!(output.contains("pragma solidity"), "should contain pragma");
    // Should contain contract
    assert!(output.contains("contract Counter"), "should contain Counter contract");
    // Should contain source file comment
    assert!(output.contains("// src/Counter.sol"), "should contain file comment");
    // Should NOT contain import statements
    assert!(!output.contains("import"), "should not contain import statements");
});

// Test that `forge flatten` resolves imports and produces a single file.
forgetest!(flatten_with_imports, |prj, cmd| {
    prj.add_source(
        "Lib.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#,
    );

    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {MathLib} from "./Lib.sol";

contract Counter {
    using MathLib for uint256;

    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function addToNumber(uint256 value) public {
        number = number.add(value);
    }
}
"#,
    );

    let output =
        cmd.args(["flatten", "src/Counter.sol"]).assert_success().get_output().stdout_lossy();

    // Should contain combined pragma
    assert!(output.contains("pragma solidity"), "should contain pragma");
    // Should contain MathLib
    assert!(output.contains("library MathLib"), "should contain MathLib library");
    // Should contain Counter
    assert!(output.contains("contract Counter"), "should contain Counter contract");
    // Should NOT contain import statements
    assert!(!output.contains("import"), "should not contain import statements");
    // Should contain file comments for both sources
    assert!(output.contains("// src/Lib.sol"), "should contain Lib.sol file comment");
    assert!(output.contains("// src/Counter.sol"), "should contain Counter.sol file comment");
    // Lib should appear before Counter (dependency ordering)
    let lib_pos = output.find("// src/Lib.sol").unwrap();
    let counter_pos = output.find("// src/Counter.sol").unwrap();
    assert!(lib_pos < counter_pos, "Lib.sol should appear before Counter.sol");
});

// Test that `forge flatten` produces output to a file with `--output`.
forgetest!(flatten_output_to_file, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;
}
"#,
    );

    let output_path = prj.root().join("flat.sol");

    cmd.args(["flatten", "src/Counter.sol", "--output", output_path.to_str().unwrap()])
        .assert_success();

    let content = std::fs::read_to_string(&output_path).unwrap();
    assert!(content.contains("contract Counter"), "output file should contain contract");
});

// Test that `forge flatten` does not use Solc (no compilation step).
// The flattening should work purely with Solar-based parsing via the dependency graph.
forgetest!(flatten_no_solc_required, |prj, cmd| {
    prj.add_source(
        "Simple.sol",
        r#"
pragma solidity ^0.8.13;

contract Simple {
    uint256 public x;
}
"#,
    );

    // Should succeed without any solc compilation artifacts
    let output =
        cmd.args(["flatten", "src/Simple.sol"]).assert_success().get_output().stdout_lossy();

    assert!(output.contains("contract Simple"), "should flatten without solc");
});
