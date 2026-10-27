//! ERC-7201 storage inspection through the CLI.

use foundry_compilers::artifacts::output_selection::ContractOutputSelection;

#[forgetest]
fn erc7201_storage_layout(prj: _, cmd: _) {
    prj.add_source(
        "Namespaced.sol",
        r#"
        contract Base {
            /// @custom:storage-location erc7201:openzeppelin.storage.Initializable
            struct Data { uint64 initialized; bool initializing; }
        }
        contract Left is Base {}
        contract Right is Base {}
        contract Namespaced is Left, Right { uint256 normal; }
        contract Unrelated {
            /// @custom:storage-location erc7201:openzeppelin.storage.Initializable
            struct Data { uint256 ignored; }
        }
    "#,
    );
    // Both fresh and cached project resolution must retain inherited namespaces exactly once.
    for attempt in 0..2 {
        if attempt == 1 {
            cmd.forge_fuse().args(["build", "--extra-output", "storageLayout"]).assert_success();
        }
        cmd.forge_fuse().args(["inspect", "Namespaced", "storageLayout", "--json"])
            .assert_json_stdout(str![[r#"
{
  "storage": [
    { "astId": "{...}", "contract": "src/Namespaced.sol:Namespaced", "label": "normal", "offset": 0, "slot": "0", "type": "t_uint256" },
    { "astId": "{...}", "contract": "src/Namespaced.sol:Base", "label": "openzeppelin.storage.Initializable.initialized", "offset": 0, "slot": "108904022758810753673719992590105913556127789646572562039383141376366747609600", "type": "erc7201(openzeppelin.storage.Initializable)::t_uint64" },
    { "astId": "{...}", "contract": "src/Namespaced.sol:Base", "label": "openzeppelin.storage.Initializable.initializing", "offset": 8, "slot": "108904022758810753673719992590105913556127789646572562039383141376366747609600", "type": "erc7201(openzeppelin.storage.Initializable)::t_bool" }
  ],
  "types": {
    "t_uint256": { "encoding": "inplace", "label": "uint256", "numberOfBytes": "32" },
    "erc7201(openzeppelin.storage.Initializable)::t_uint64": { "encoding": "inplace", "label": "uint64", "numberOfBytes": "8" },
    "erc7201(openzeppelin.storage.Initializable)::t_bool": { "encoding": "inplace", "label": "bool", "numberOfBytes": "1" }
  }
}
"#]]);
    }
    cmd.forge_fuse()
        .args(["inspect", "Namespaced", "storageLayout", "--md"])
        .assert_success()
        .stdout_eq(str![[r#"

| Name                                            | Type    | Slot                                                                           | Offset | Bytes | Contract                      |
|-------------------------------------------------|---------|--------------------------------------------------------------------------------|--------|-------|-------------------------------|
| normal                                          | uint256 | 0                                                                              | 0      | 32    | src/Namespaced.sol:Namespaced |
| openzeppelin.storage.Initializable.initialized  | uint64  | 108904022758810753673719992590105913556127789646572562039383141376366747609600 | 0      | 8     | src/Namespaced.sol:Base       |
| openzeppelin.storage.Initializable.initializing | bool    | 108904022758810753673719992590105913556127789646572562039383141376366747609600 | 8      | 1     | src/Namespaced.sol:Base       |


"#]]);
    cmd.forge_fuse()
        .args(["inspect", "Namespaced", "transientStorageLayout", "--json"])
        .assert_json_stdout(str![[r#"{"storage": [], "types": {}}"#]]);
}

#[forgetest]
fn erc7201_duplicate_namespace(prj: _, cmd: _) {
    prj.add_source(
        "Duplicate.sol",
        r#"
        contract Base {
            /// @custom:storage-location erc7201:example
            struct Data { uint256 value; }
        }
        contract Duplicate is Base {
            /// @custom:storage-location erc7201:example
            struct Other { uint256 value; }
        }
    "#,
    );
    cmd.args(["inspect", "Duplicate", "storageLayout", "--json"])
        .assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: Duplicate ERC-7201 namespace `example` in `src/Duplicate.sol:Base.Data` and `src/Duplicate.sol:Duplicate.Other`

"#]]);
}

#[forgetest]
fn erc7201_malformed_namespace(prj: _, cmd: _) {
    prj.add_source(
        "Malformed.sol",
        r#"
        contract Malformed {
            /// @custom:storage-location erc7201:
            struct Data { uint256 value; }
        }
    "#,
    );
    cmd.args(["inspect", "Malformed", "storageLayout", "--json"])
        .assert_failure()
        .stdout_eq("")
        .stderr_eq(str![[r#"
Error: ERC-7201 namespace must be nonempty and contain no whitespace

"#]]);
}

// Namespaces of contracts whose sources weren't recompiled are still decoded in state diffs.
#[forgetest]
fn erc7201_state_diff_after_partial_recompile(prj: _, cmd: _) {
    prj.update_config(|config| config.extra_output = vec![ContractOutputSelection::StorageLayout]);
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    /// @custom:storage-location erc7201:example.counter
    struct CounterStorage { uint256 count; }

    function increment() external {
        CounterStorage storage $;
        bytes32 slot = keccak256(abi.encode(uint256(keccak256("example.counter")) - 1)) & ~bytes32(uint256(0xff));
        assembly { $.slot := slot }
        $.count += 1;
    }
}
"#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import "../src/Counter.sol";

interface Vm {
    function startStateDiffRecording() external;
    function getStateDiffJson() external returns (string memory);
    function contains(string calldata subject, string calldata search) external returns (bool);
}

contract CounterTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testNamespaceLabel() public {
        Counter counter = new Counter();
        vm.startStateDiffRecording();
        counter.increment();
        require(vm.contains(vm.getStateDiffJson(), '"label":"example.counter.count"'));
    }
}
"#,
    );
    prj.add_test("Other.t.sol", "contract OtherTest { function testOther() public {} }");
    cmd.arg("test").assert_success();

    // Only `Other.t.sol` is recompiled.
    prj.add_test("Other.t.sol", "contract OtherTest { function testOther() public pure {} }");
    cmd.forge_fuse().arg("test").assert_success();
}
