//! Cheatcode access for libraries explicitly linked to forked tests and scripts.

use super::*;

#[forgetest]
async fn fork_configured_libraries_have_cheatcode_access(prj: _, cmd: _) {
    prj.add_source("CheatcodeLibrary.sol", LIBRARY_SOURCE);
    prj.add_test("CheatcodeLibrary.t.sol", LIBRARY_TEST);
    prj.add_script("CheatcodeLibrary.s.sol", LIBRARY_SCRIPT);
    prj.update_config(|config| {
        config.libraries = vec![
            "src/CheatcodeLibrary.sol:ConfiguredLibrary:0x0000000000000000000000000000000000001234"
                .into(),
            "src/CheatcodeLibrary.sol:UnusedLibrary:0x0000000000000000000000000000000000005678"
                .into(),
        ];
    });
    let output = cmd
        .args(["inspect", "src/CheatcodeLibrary.sol:ConfiguredLibrary", "deployedBytecode"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let code = output.trim().parse::<Bytes>().unwrap();
    let (api, handle) = spawn(NodeConfig::test()).await;
    for target in [
        address!("0000000000000000000000000000000000001234"),
        address!("0000000000000000000000000000000000005678"),
        address!("0000000000000000000000000000000000009012"),
    ] {
        api.anvil_set_code(target, code.clone()).await.unwrap();
    }
    cmd.unset_env("FOUNDRY_LINKED_LIBRARY_UNUSED");
    cmd.forge_fuse()
        .args(["test", "--fork-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CheatcodeLibrary.t.sol:CheatcodeLibraryTest
[PASS] testForkConfiguredLibraryAccess() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
    cmd.forge_fuse()
        .args(["script", "CheatcodeLibraryScript", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
...
Script ran successfully.
...
"#]]);
}

const LIBRARY_SOURCE: &str = r#"
interface Vm {
    function envOr(string calldata name, uint256 defaultValue) external view returns (uint256);
}

library ConfiguredLibrary {
    function read() external view returns (uint256) {
        return Vm(address(uint160(uint256(keccak256("hevm cheat code")))))
            .envOr("FOUNDRY_LINKED_LIBRARY_UNUSED", uint256(7));
    }
}

library UnusedLibrary {
    function read() external pure returns (uint256) {
        return 0;
    }
}

abstract contract LibraryConsumer {
    constructor() {
        require(ConfiguredLibrary.read() == 7, "constructor library access lost");
    }

    function checkAccess() internal {
        require(ConfiguredLibrary.read() == 7, "runtime library access lost");
        (bool ok, bytes memory output) = address(0x5678).delegatecall(abi.encodeWithSignature("read()"));
        require(ok && abi.decode(output, (uint256)) == 7, "explicitly configured library access lost");

        (ok, output) = address(0x9012).delegatecall(abi.encodeWithSignature("read()"));
        require(!ok, "unconfigured code unexpectedly authorized");
        bytes memory expected = abi.encodeWithSignature(
            "CheatcodeError(string)",
            "cheatcodes are not enabled for 0x0000000000000000000000000000000000009012; see `vm.allowCheatcodes(address)`"
        );
        require(keccak256(output) == keccak256(expected), "unexpected authorization error");
    }
}
"#;

const LIBRARY_TEST: &str = r#"
import "../src/CheatcodeLibrary.sol";

contract CheatcodeLibraryTest is LibraryConsumer {
    function testForkConfiguredLibraryAccess() public {
        checkAccess();
    }
}
"#;

const LIBRARY_SCRIPT: &str = r#"
import "../src/CheatcodeLibrary.sol";

contract CheatcodeLibraryScript is LibraryConsumer {
    function run() public {
        checkAccess();
    }
}
"#;
