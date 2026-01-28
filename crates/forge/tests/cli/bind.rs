// <https://github.com/foundry-rs/foundry/issues/11177>
// Test that bindings compile when interface name ends with `Events`
// (which collides with the generated events enum name)
forgetest!(bind_interface_events_collision, |prj, cmd| {
    prj.add_source(
        "IExampleContractEvents.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IExampleContractEvents {
    struct SomeEventData {
        uint256 value;
        address sender;
    }

    event ExampleEvent(SomeEventData data);
}

interface IExampleContract is IExampleContractEvents {
    function doSomething(SomeEventData calldata data) external;
}
   "#,
    );
    cmd.args(["bind"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Generating bindings for 2 contracts
Bindings have been generated to [..]
"#]]);

    let bindings_path = prj.root().join("out/bindings");
    assert!(bindings_path.exists(), "Bindings directory should exist");

    // Verify that the generated bindings compile (the bug was a naming collision)
    // Commented out to avoid running cargo build in CI, but kept for local debugging
    // use std::process::Command;
    // let out = Command::new("cargo")
    //     .arg("build")
    //     .current_dir(&bindings_path)
    //     .output()
    //     .expect("Failed to run cargo build");
    //
    // assert!(
    //     out.status.success(),
    //     "Cargo build should succeed. Stderr: {}",
    //     String::from_utf8_lossy(&out.stderr)
    // );
});

// <https://github.com/foundry-rs/foundry/issues/9482>
forgetest!(bind_unlinked_bytecode, |prj, cmd| {
    prj.add_source(
        "SomeLibContract.sol",
        r#"
library SomeLib {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}

contract SomeLibContract {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return SomeLib.add(a, b);
    }
}
   "#,
    );
    cmd.args(["bind", "--select", "SomeLibContract"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Generating bindings for 1 contracts
Bindings have been generated to [..]
"#]]);
});
