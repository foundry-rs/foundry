//! Tests for commands using the preprocessed cache.

use foundry_compilers::artifacts::{EvmVersion, remappings::Remapping};
use foundry_config::{CompilationRestrictions, SettingsOverrides};

#[cfg(unix)]
use foundry_compilers::artifacts::{SolcInput, output_selection::OutputSelection};

// <https://github.com/foundry-rs/foundry/issues/16852>
forgetest!(preprocess_parenthesized_new, |prj, cmd| {
    prj.add_source(
        "Target.sol",
        r#"
contract Empty {
    constructor() payable {}
}
contract Target {
    uint256 public immutable n;
    constructor(uint256 number) payable { n = number; }
}
"#,
    );
    prj.add_test(
        "Value.t.sol",
        r#"
import {Empty, Target} from "../src/Target.sol";
contract ValueTest {
    function test_empty() public {
        Empty a = (new Empty){value: 1 ether}();
        Empty b = ((new Empty)){value: 2 ether}();
        Empty c = (new Empty)();
        Empty d = new Empty{value: 3 ether}();
        require(address(a).balance == 1 ether);
        require(address(b).balance == 2 ether);
        require(address(c).balance == 0);
        require(address(d).balance == 3 ether);
    }
    function test_arguments() public {
        Target a = (new Target){value: 1 ether}(42);
        Target b = ((new Target)){value: 2 ether}({number: 7});
        Target c = ((new Target))(8);
        Target d = (new Target{value: 3 ether})(9);
        require(a.n() == 42 && address(a).balance == 1 ether);
        require(b.n() == 7 && address(b).balance == 2 ether);
        require(c.n() == 8 && address(c).balance == 0);
        require(d.n() == 9 && address(d).balance == 3 ether);
    }
    function test_salt() public {
        Target target = (new Target){salt: bytes32(uint256(1)), value: 1 ether}(42);
        address expected = address(uint160(uint256(keccak256(abi.encodePacked(
            bytes1(0xff), address(this), bytes32(uint256(1)),
            keccak256(abi.encodePacked(type(Target).creationCode, abi.encode(uint256(42))))
        )))));
        require(address(target) == expected);
        require(target.n() == 42 && address(target).balance == 1 ether);
    }
    function test_try() public {
        try (new Target){value: 1 ether}(42) returns (Target target) {
            require(target.n() == 42 && address(target).balance == 1 ether);
        } catch { revert("deployment failed"); }
    }
}
"#,
    );
    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        cmd.forge_fuse().args(["test", "--force"]).assert_success().stdout_eq(str![[r#"
...
Ran 4 tests for test/Value.t.sol:ValueTest
[PASS] test_arguments() ([GAS])
[PASS] test_empty() ([GAS])
[PASS] test_salt() ([GAS])
[PASS] test_try() ([GAS])
Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]);
    }

    cmd.forge_fuse()
        .args(["test", "--match-test", "test_arguments", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/Value.t.sol:ValueTest
[PASS] test_arguments() ([GAS])
Traces:
  [[..]] ValueTest::test_arguments()
    ├─ [0] VM::deployCode("src/Target.sol:Target", 0x000000000000000000000000000000000000000000000000000000000000002a, 1000000000000000000 [1e18])
    │   ├─ [[..]] → new Target@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   │   └─ ← [Return] 203 bytes of code
    │   └─ ← [Return] Target: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f]
    ├─ [0] VM::deployCode("src/Target.sol:Target", 0x0000000000000000000000000000000000000000000000000000000000000007, 2000000000000000000 [2e18])
    │   ├─ [[..]] → new Target@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   │   └─ ← [Return] 203 bytes of code
    │   └─ ← [Return] Target: [0x2e234DAe75C793f67A35089C9d99245E1C58470b]
    ├─ [0] VM::deployCode("src/Target.sol:Target", 0x0000000000000000000000000000000000000000000000000000000000000008)
    │   ├─ [[..]] → new Target@0xF62849F9A0B5Bf2913b396098F7c7019b51A820a
    │   │   └─ ← [Return] 203 bytes of code
    │   └─ ← [Return] Target: [0xF62849F9A0B5Bf2913b396098F7c7019b51A820a]
    ├─ [0] VM::deployCode("src/Target.sol:Target", 0x0000000000000000000000000000000000000000000000000000000000000009, 3000000000000000000 [3e18])
    │   ├─ [[..]] → new Target@0x5991A2dF15A8F6A256D3Ec51E99254Cd3fb576A9
    │   │   └─ ← [Return] 203 bytes of code
    │   └─ ← [Return] Target: [0x5991A2dF15A8F6A256D3Ec51E99254Cd3fb576A9]
    ├─ [303] Target::n() [staticcall]
    │   └─ ← [Return] 42
    ├─ [303] Target::n() [staticcall]
    │   └─ ← [Return] 7
    ├─ [303] Target::n() [staticcall]
    │   └─ ← [Return] 8
    ├─ [303] Target::n() [staticcall]
    │   └─ ← [Return] 9
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapped_bytecode_dependencies, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@p/=src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
pragma solidity ^0.8.0;
contract Impl {
    constructor(uint256) {}
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl(1).v() == 111, "stale implementation");
    }
    function test_creationCode() public {
        bytes memory code = abi.encodePacked(type(Impl).creationCode, abi.encode(uint256(1)));
        address deployed;
        assembly { deployed := create(0, add(code, 32), mload(code)) }
        require(Impl(deployed).v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.env("RUST_LOG", "error");
    cmd.args(["test"]).assert_success().stderr_eq("").stdout_eq(str![[r#"
...
Ran 2 tests for test/Impl.t.sol:ImplTest
[PASS] test_creationCode() ([GAS])
[PASS] test_new() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // A body-only edit must reach both dynamically linked bytecode references.
    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    prj.forge_command().arg("build").with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 1 files with [..]
[..]
Compiler run successful!

"#]]);
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_creationCode() ([GAS])
[FAIL: stale implementation] test_new() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_creationCode() ([GAS])
[FAIL: stale implementation] test_new() ([GAS])

Encountered a total of 2 failing tests, 0 tests succeeded
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16901>
forgetest!(preprocess_external_bytecode_dependencies, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@dep/=lib/dep/src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
pragma solidity ^0.8.0;
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.create_file("lib/dep/src/RemappedImpl.sol", source);
    prj.create_file("external/RelativeImpl.sol", source);
    prj.add_test(
        "ExternalImpl.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl as RemappedImpl} from "@dep/RemappedImpl.sol";
import {Impl as RelativeImpl} from "../external/RelativeImpl.sol";
contract ExternalImplTest {
    function test_remapped() public {
        require(new RemappedImpl().v() == 111, "stale remapped implementation");
    }
    function test_relative() public {
        require(new RelativeImpl().v() == 111, "stale relative implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    // External deployments remain native, so body-only edits must rebuild their importer.
    let changed = source.replace("return 111", "return 222");
    prj.create_file("lib/dep/src/RemappedImpl.sol", &changed);
    prj.create_file("external/RelativeImpl.sol", &changed);
    cmd.forge_fuse().arg("test").with_no_redact().assert_failure().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!
...
[FAIL: stale relative implementation] test_relative() ([..])
[FAIL: stale remapped implementation] test_remapped() ([..])
...
"#]]);
});

forgetest!(preprocess_external_dependencies_invalidate_independently, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@dep/=lib/dep/src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.create_file("lib/dep/src/RemappedImpl.sol", source);
    prj.create_file("external/RelativeImpl.sol", source);
    let remapped_test = |expected| {
        format!(
            r#"
pragma solidity ^0.8.0;
import {{Impl}} from "@dep/RemappedImpl.sol";
contract RemappedTest {{
    function test_remapped() public {{
        require(new Impl().v() == {expected}, "stale remapped implementation");
    }}
}}
"#,
        )
    };
    prj.add_test("Remapped.t.sol", &remapped_test(111));
    prj.add_test(
        "Relative.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../external/RelativeImpl.sol";
contract RelativeTest {
    function test_relative() public {
        require(new Impl().v() == 111, "stale relative implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    let changed = source.replace("return 111", "return 222");
    prj.create_file("lib/dep/src/RemappedImpl.sol", &changed);
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale remapped implementation] test_remapped() ([..])
...
"#]]);

    // Make the first importer green without touching the second, then verify the relative edge.
    prj.add_test("Remapped.t.sol", &remapped_test(222));
    prj.create_file("external/RelativeImpl.sol", &changed);
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale relative implementation] test_relative() ([..])
...
"#]]);
});

forgetest!(preprocess_native_bytecode_forms, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Native.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
interface Vm { function etch(address, bytes calldata) external; }
function make() returns (Impl) { return new Impl(); }
contract NativeTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    function test_runtime_code() public {
        address target = address(0xBEEF);
        vm.etch(target, type(Impl).runtimeCode);
        (bool ok, bytes memory out) = target.staticcall(abi.encodeCall(Impl.v, ()));
        require(ok && abi.decode(out, (uint256)) == 111, "stale runtime bytecode");
    }
    function test_free_function() public {
        require(make().v() == 111, "stale free function bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale free function bytecode] test_free_function() ([..])
[FAIL: stale runtime bytecode] test_runtime_code() ([..])
...
"#]]);
});

forgetest!(preprocess_same_file_free_function_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "FreeFunction.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
function make() returns (Impl) { return new Impl(); }
contract FreeFunctionTest {
    function test_free_function() public {
        require(make().v() == 111, "stale free function bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale free function bytecode] test_free_function() ([..])
...
"#]]);
});

forgetest!(preprocess_script_native_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_script(
        "Native.s.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
contract NativeScript {
    function run() public {
        require(new Impl().v() == 111, "stale script bytecode");
    }
}
"#,
    );
    cmd.args(["script", "script/Native.s.sol:NativeScript"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().args(["script", "script/Native.s.sol:NativeScript"]).assert_failure();
});

forgetest!(preprocess_imported_free_function_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "Factory.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
/*
This padding deliberately places the imported function's expression beyond the end of the test
source. Recursive dependency analysis must use the callee's source map rather than slicing the
importer with the callee's offsets.
....................................................................................................
....................................................................................................
....................................................................................................
*/
function make() returns (Impl) { return new Impl(); }
"#,
    );
    prj.add_test(
        "FreeFunction.t.sol",
        r#"
pragma solidity ^0.8.0;
import {make} from "../src/Factory.sol";
contract FreeFunctionTest {
    function test_free_function() public {
        require(make().v() == 111, "stale imported free function bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale imported free function bytecode] test_free_function() ([..])
...
"#]]);
});

forgetest!(preprocess_function_reference_dependencies, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "Factory.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
function makeRef() returns (Impl) { return new Impl(); }
function make() returns (Impl) { return new Impl(); }
function make(uint256) returns (Impl) { return new Impl(); }
"#,
    );
    prj.add_test(
        "FunctionReference.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
import {make, makeRef} from "../src/Factory.sol";
contract FunctionReferenceTest {
    function test_function_reference() public {
        function () internal returns (Impl) factory = makeRef;
        require(factory().v() == 111, "stale function reference bytecode");
    }
    function test_overloaded_function() public {
        require(make().v() == 111, "stale overloaded function bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale function reference bytecode] test_function_reference() ([..])
[FAIL: stale overloaded function bytecode] test_overloaded_function() ([..])
...
"#]]);
});

forgetest!(preprocess_shared_free_function_is_not_rewritten, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    prj.add_source("Impl.sol", "contract Impl {}");
    prj.add_test(
        "SharedFreeFunction.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
function make() returns (Impl) { return new Impl(); }
contract FirstTest {
    function test_first() public { require(address(make()) != address(0)); }
}
contract SecondTest {
    function test_second() public { require(address(make()) != address(0)); }
}
"#,
    );

    cmd.args(["test"]).assert_success();
});

forgetest!(preprocess_try_call_option_dependency_is_not_rewritten, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    prj.add_source("Created.sol", "contract Created {}");
    prj.add_source("Salt.sol", "contract Salt {}");
    prj.add_test(
        "TryCallOption.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Created} from "../src/Created.sol";
import {Salt} from "../src/Salt.sol";
contract TryCallOptionTest {
    function test_try_call_option() public {
        try new Created{salt: keccak256(type(Salt).creationCode)}() returns (Created created) {
            require(address(created) != address(0));
        } catch { revert(); }
    }
}
"#,
    );

    cmd.args(["test"]).assert_success();
});

forgetest!(preprocess_internal_library_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "FactoryLib.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
library FactoryLib {
    function make() internal returns (Impl) { return new Impl(); }
}
"#,
    );
    prj.add_test(
        "Library.t.sol",
        r#"
pragma solidity ^0.8.0;
import {FactoryLib} from "../src/FactoryLib.sol";
contract LibraryTest {
    function test_library() public {
        require(FactoryLib.make().v() == 111, "stale library bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale library bytecode] test_library() ([..])
...
"#]]);
});

forgetest!(preprocess_namespace_library_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "FactoryLib.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
library FactoryLib {
    function make() internal returns (Impl) { return new Impl(); }
}
"#,
    );
    prj.add_test(
        "NamespaceLibrary.t.sol",
        r#"
pragma solidity ^0.8.0;
import "../src/FactoryLib.sol" as Factories;
contract NamespaceLibraryTest {
    function test_namespace_library() public {
        require(
            Factories.FactoryLib.make().v() == 111,
            "stale namespace library bytecode"
        );
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale namespace library bytecode] test_namespace_library() ([..])
...
"#]]);
});

forgetest!(preprocess_using_library_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "FactoryLib.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
library FactoryLib {
    function make(uint256) internal returns (Impl) { return new Impl(); }
}
"#,
    );
    prj.add_test(
        "UsingLibrary.t.sol",
        r#"
pragma solidity ^0.8.0;
import {FactoryLib} from "../src/FactoryLib.sol";
contract UsingLibraryTest {
    using FactoryLib for uint256;
    function test_using_library() public {
        require(uint256(0).make().v() == 111, "stale using library bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale using library bytecode] test_using_library() ([..])
...
"#]]);
});

forgetest!(preprocess_try_constructor_argument_dependencies, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "FactoryLib.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
library FactoryLib {
    function make() internal returns (Impl) { return new Impl(); }
}
"#,
    );
    prj.add_source(
        "Receiver.sol",
        r#"
pragma solidity ^0.8.0;
contract Receiver {
    uint256 public immutable value;
    constructor(uint256 value_) { value = value_; }
}
"#,
    );
    prj.add_test(
        "TryPlainLibrary.t.sol",
        r#"
pragma solidity ^0.8.0;
import {FactoryLib} from "../src/FactoryLib.sol";
import {Receiver} from "../src/Receiver.sol";
contract TryPlainLibraryTest {
    function test_try_plain_library() public {
        try new Receiver(FactoryLib.make().v()) returns (Receiver receiver) {
            require(receiver.value() == 111, "stale try plain library bytecode");
        } catch { revert(); }
    }
}
"#,
    );
    prj.add_test(
        "TryNamespaceLibrary.t.sol",
        r#"
pragma solidity ^0.8.0;
import "../src/FactoryLib.sol" as Factories;
import {Receiver} from "../src/Receiver.sol";
contract TryNamespaceLibraryTest {
    function test_try_namespace_library() public {
        try new Receiver(Factories.FactoryLib.make().v()) returns (Receiver receiver) {
            require(receiver.value() == 111, "stale try namespace library bytecode");
        } catch { revert(); }
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale try namespace library bytecode] test_try_namespace_library() ([..])
...
[FAIL: stale try plain library bytecode] test_try_plain_library() ([..])
...
"#]]);
});

forgetest!(preprocess_external_inheritance_dependency, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Base { function v() public pure returns (uint256) { return 111; } }
"#;
    prj.create_file("external/Base.sol", source);
    prj.add_test(
        "Inheritance.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Base} from "../external/Base.sol";
contract InheritanceTest is Base {
    function test_inherited() public pure {
        require(v() == 111, "stale inherited bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.create_file("external/Base.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale inherited bytecode] test_inherited() ([..])
...
"#]]);
});

forgetest!(preprocess_expanded_source_context_invalidates_prior_artifacts, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file("vendor/pkg/src/Impl.sol", source);
    prj.add_test(
        "A.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
contract ATest {
    function test_a() public { require(new Impl().v() == 111, "stale A bytecode"); }
}
"#,
    );
    prj.add_test(
        "B.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../vendor/pkg/src/Impl.sol";
contract BTest { function test_b() public { new Impl(); } }
"#,
    );

    cmd.args(["build", "test/A.t.sol"]).assert_success();
    cmd.forge_fuse().args(["build", "test/B.t.sol"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().args(["test", "--match-path", "test/A.t.sol"]).assert_failure().stdout_eq(
        str![[r#"
...
[FAIL: stale A bytecode] test_a() ([..])
...
"#]],
    );
});

forgetest!(preprocess_nested_absolute_imports, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["dep/=lib/dep/".parse::<Remapping>().unwrap().into()];
    });
    prj.create_file(
        "lib/dep/src/Child.sol",
        r#"
pragma solidity ^0.8.0;
contract Child {}
"#,
    );
    prj.create_file(
        "lib/dep/src/Base.sol",
        r#"
pragma solidity ^0.8.0;
import "src/Child.sol";
contract Base { function make() external returns (Child) { return new Child(); } }
"#,
    );
    prj.add_test(
        "Nested.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Base} from "dep/src/Base.sol";
contract NestedTest { function test_nested() public { new Base(); } }
"#,
    );

    cmd.args(["test"]).assert_success();
});

forgetest!(preprocess_analysis_failure_is_conservative, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
pragma solidity ^0.8.0;
contract Impl { function v() external pure returns (uint256) { return 111; } }
"#;
    prj.add_source("Impl.sol", source);
    prj.add_source(
        "Derived.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "./Impl.sol";
contract Derived is Impl {}
"#,
    );
    prj.add_test(
        "Fallback.t.sol",
        r#"
pragma solidity ^0.8.0;
import {Impl} from "../src/Impl.sol";
import {Derived} from "../src/Derived.sol";
contract FallbackTest {
    function test_fallback() public {
        new Derived();
        require(new Impl().v() == 111, "stale conservative bytecode");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale conservative bytecode] test_fallback() ([..])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapping_identity, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = ["@p/=src/../src/", "src/=lib/alternate/"]
            .into_iter()
            .map(|remapping| remapping.parse::<Remapping>().unwrap().into())
            .collect();
    });
    let source = r#"
contract Impl {
    struct Args { uint256 value; }
    constructor(Args memory args) {}
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file(
        "lib/alternate/Impl.sol",
        r#"
contract Impl {
    struct Args { address value; }
    constructor(Args memory args) {}
    function v() external pure returns (uint256) { return 999; }
}
"#,
    );
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl as Implementation} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(
            new Implementation(Implementation.Args({value: 1})).v() == 111,
            "stale implementation"
        );
    }
}
contract EmptyTest {}
"#,
    );
    cmd.args(["test"]).assert_success();

    // Ambiguous source-unit references stay native and are invalidated after a body-only edit.
    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").with_no_redact().assert_failure().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!

Ran 1 test for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; finished in [..] ([..] CPU time)

Ran 1 test suite in [..] ([..] CPU time): 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])

Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapping_context_uses_running_test, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings =
            vec!["test/suite/:src/=lib/alternate/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file("lib/alternate/Impl.sol", &source.replace("return 111", "return 999"));
    prj.add_test(
        "support/Helper.sol",
        r#"
import {Impl} from "src/Impl.sol";
contract Helper {
    function create() public returns (Impl) { return new Impl(); }
}
"#,
    );
    prj.add_test(
        "suite/Impl.t.sol",
        r#"
import {Helper} from "../support/Helper.sol";
contract ImplTest is Helper {
    function test_new() public {
        require(create().v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale implementation] test_new() ([GAS])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapped_helper_source, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = ["@p/=src/", "test/:foundry-pp/=lib/alternate/"]
            .into_iter()
            .map(|remapping| remapping.parse::<Remapping>().unwrap().into())
            .collect();
    });
    let source = r#"
contract Impl {
    constructor(uint256) {}
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl(1).v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale implementation] test_new() ([GAS])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_ambiguous_artifact_stays_native, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file("vendor/pkg/src/Impl.sol", source);
    prj.add_source(
        "UsesLib.sol",
        r#"
import {Impl as LibImpl} from "vendor/pkg/src/Impl.sol";
contract UsesLib {
    function create() public returns (LibImpl) { return new LibImpl(); }
}
"#,
    );
    prj.forge_command().arg("build").assert_success();

    let test = r#"
import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl().v() == 111, "stale implementation");
    }
}
"#;
    prj.add_test("Impl.t.sol", test);
    cmd.args(["test"]).assert_success();

    // A genuinely narrower request must retain the native fallback classification.
    prj.add_test("Impl.t.sol", &format!("\n{test}"));
    cmd.forge_fuse().args(["build", "test/Impl.t.sol"]).assert_success();
    cmd.forge_fuse().arg("test").assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale implementation] test_new() ([GAS])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapped_mock_inheritance, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@p/=src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    function v() public pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "ImplMock.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplMock is Impl {}
"#,
    );
    prj.add_test(
        "Impl.t.sol",
        r#"
import {ImplMock} from "./ImplMock.sol";
contract ImplTest is ImplMock {
    function test_inherited() public pure {
        require(v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Impl.t.sol:ImplTest
[PASS] test_inherited() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Inherited bytecode stays native, so the test must be rebuilt as well.
    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    prj.forge_command().arg("build").with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!

"#]]);
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_inherited() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_inherited() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
#[cfg(unix)]
forgetest!(preprocess_remapped_symlinked_source, |prj, cmd| {
    use std::{fs, os::unix::fs::symlink};

    fs::remove_dir_all(prj.root().join("src")).unwrap();
    fs::create_dir_all(prj.root().join(".shared/src")).unwrap();
    symlink(".shared/src", prj.root().join("src")).unwrap();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@p/=src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl().v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").with_no_redact().assert_failure().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!

Ran 1 test for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; finished in [..] ([..] CPU time)

Ran 1 test suite in [..] ([..] CPU time): 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])

Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
});

#[cfg(unix)]
forgetest_init!(abi_commands_reuse_preprocessed_cache, |prj, cmd| {
    use foundry_test_utils::util::OutputExt;
    use std::{fs, os::unix::fs::PermissionsExt};

    prj.initialize_default_contracts();
    prj.update_config(|config| config.dynamic_test_linking = true);
    cmd.arg("build").assert_success();

    let solc = prj.root().join("fake-solc");
    let invoked = prj.root().join("fake-solc.invoked");
    fs::write(
        &solc,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
    echo "solc, the solidity compiler commandline interface"
    echo "Version: 0.8.35+commit.69074fbd"
    exit 0
fi
cat > "$0.invoked"
exit 1
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&solc).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&solc, permissions).unwrap();
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Local(solc.clone()));
    });

    let output =
        cmd.forge_fuse().args(["test", "--match-contract", "CounterTest"]).assert_success();
    let stdout = output.get_output().stdout_lossy();
    assert!(
        stdout.contains("Ran 2 tests for test/Counter.t.sol:CounterTest"),
        "cached ABI did not select CounterTest: {stdout}"
    );
    assert!(!invoked.exists(), "filtered test compilation did not reuse the preprocessed cache");

    cmd.forge_fuse().args(["selectors", "list"]).assert_success();
    assert!(!invoked.exists(), "selector compilation did not reuse the preprocessed cache");

    // A new, unselected test is available only through discovery's secondary cache.
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Version(
            foundry_test_utils::util::SOLC_VERSION.parse().unwrap(),
        ));
    });
    prj.add_test("Other.t.sol", "contract OtherTest { function test_other() public {} }");
    cmd.forge_fuse().args(["test", "--match-contract", "CounterTest"]).assert_success();
    let abi_cache = prj.cache().with_file_name("solidity-files-cache.json.abi");
    assert!(abi_cache.is_dir());
    assert!(!prj.artifacts().join("Other.t.sol").exists());
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Local(solc));
    });
    cmd.forge_fuse().args(["test", "--match-contract", "CounterTest"]).assert_success();
    assert!(!invoked.exists(), "partial-cache discovery invoked solc");

    // Disabling caching must bypass both stores, even after warming them.
    prj.update_config(|config| config.cache = false);
    cmd.forge_fuse().args(["test", "--match-contract", "CounterTest"]).assert_failure();
    assert!(invoked.exists(), "cache=false reused cached discovery");
    let input = serde_json::from_slice::<SolcInput>(&fs::read(&invoked).unwrap()).unwrap();
    // A bytecode compile could also fail here; prove that discovery itself invoked Solc.
    let expected = OutputSelection::common_output_selection(["abi".to_string()]);
    assert!(!input.settings.output_selection.0.is_empty());
    for selection in input.settings.output_selection.0.values() {
        assert_eq!(
            selection, &expected.0["*"],
            "cache=false must recompile ABI discovery before attempting bytecode compilation",
        );
    }
    cmd.forge_fuse().arg("clean").assert_success();
    assert!(!abi_cache.exists());
});

// <https://github.com/foundry-rs/foundry/issues/8842>
forgetest_init!(filtered_tests_compile_unimported_test_fixtures, |prj, cmd| {
    prj.update_config(|config| config.solc = None);
    prj.add_raw_test(
        "fixtures/Fixture.sol",
        r#"
pragma solidity 0.7.6;

contract Fixture {
    function version() external pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface IFixture {
    function version() external pure returns (uint256);
}

contract FixtureTest is Test {
    function testFixture() public {
        address fixture = vm.deployCode("test/fixtures/Fixture.sol:Fixture");
        assertEq(IFixture(fixture).version(), 1);
    }
}
"#,
    );

    cmd.args(["test", "--match-contract", "FixtureTest"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    prj.add_raw_test(
        "fixtures/Fixture.sol",
        r#"
pragma solidity 0.7.6;

contract Fixture {
    function version() external pure returns (uint256) {
        return 2;
    }
}
"#,
    );
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface IFixture {
    function version() external pure returns (uint256);
}

contract FixtureTest is Test {
    function testFixture() public {
        address fixture = vm.deployCode("test/fixtures/Fixture.sol:Fixture");
        assertEq(IFixture(fixture).version(), 2);
    }
}
"#,
    );

    cmd.assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/8842>
forgetest_init!(path_filtered_tests_compile_unimported_test_fixtures, |prj, cmd| {
    prj.update_config(|config| {
        config.solc = None;
        config.dynamic_test_linking = false;
    });
    prj.add_raw_script("Broken.s.sol", "this is not valid Solidity");
    prj.add_raw_test(
        "fixtures/Fixture.sol",
        r#"
pragma solidity 0.7.6;

contract Fixture {}
"#,
    );
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FixtureTest is Test {
    function testFixture() public {
        assertGt(vm.getCode("test/fixtures/Fixture.sol:Fixture").length, 0);
    }
}
"#,
    );

    cmd.args(["test", "--match-path", "test/Fixture.t.sol"]).assert_success().stdout_eq(str![[
        r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#
    ]]);
});

// <https://github.com/foundry-rs/foundry/issues/16529>
forgetest_init!(filtered_tests_preserve_compilation_restrictions, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_lib(
        "dep/src/Clz.sol",
        r#"
library Clz {
    function msb(uint128 bitmap) internal pure returns (uint256 res) {
        assembly {
            res := sub(255, clz(bitmap))
        }
    }
}
"#,
    );
    prj.add_source(
        "Root.sol",
        r#"
import "../lib/dep/src/Clz.sol";

contract Root {
    function msb(uint128 bitmap) external pure returns (uint256) {
        return Clz.msb(bitmap);
    }
}
"#,
    );
    prj.add_test("RootTest.sol", "contract RootTest { function testFoo() public pure {} }");
    prj.update_config(|config| {
        config.evm_version = EvmVersion::Prague;
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "osaka".to_string(),
            via_ir: None,
            evm_version: Some(EvmVersion::Osaka),
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "src/Root.sol".parse().unwrap(),
            version: None,
            via_ir: None,
            bytecode_hash: None,
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: Some(EvmVersion::Osaka),
            max_evm_version: None,
        }];
    });

    cmd.args(["test", "--match-path", "test/RootTest.sol"]).assert_success();
});

forgetest_init!(filtered_tests_support_overlapping_source_roots, |prj, cmd| {
    prj.update_config(|config| {
        config.script = ".".into();
        config.dynamic_test_linking = true;
    });
    prj.add_source("SourceFixture.sol", "contract SourceFixture {}");
    prj.add_source(
        "Child.sol",
        "contract Child { function value() external pure returns (uint256) { return 1; } }",
    );
    prj.add_source(
        "Factory.sol",
        "import {Child} from './Child.sol'; contract Factory { function create() external returns (Child) { return new Child(); } }",
    );
    prj.add_test("fixtures/Fixture.sol", "contract Fixture {}");
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Child} from "../src/Child.sol";
import {Factory} from "../src/Factory.sol";

contract FixtureTest is Test {
    function testFixture() public {
        assertGt(vm.getCode("test/fixtures/Fixture.sol:Fixture").length, 0);
        assertGt(vm.getCode("src/SourceFixture.sol:SourceFixture").length, 0);
        Factory factory = new Factory();
        Child child = factory.create();
        assertEq(child.value(), 1);
    }
}
"#,
    );

    cmd.args(["test", "--match-contract", "FixtureTest"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Test cache is invalidated when `forge build` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_build, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });
    // All files are built with optimized tests.
    cmd.args(["build"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...

"#]]);
    // No files are rebuilt.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Toggle test optimizer off.
    prj.update_config(|config| {
        config.dynamic_test_linking = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...

"#]]);
});

// Test cache is invalidated when `forge test` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_test, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });
    // All files are built with optimized tests.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
    // No files are rebuilt.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Toggle test optimizer off.
    prj.update_config(|config| {
        config.dynamic_test_linking = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16468>
forgetest_init!(unchecked_artifacts_support_dynamic_linking, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.unchecked_cheatcode_artifacts = true;
    });
    prj.add_source(
        "Counter.sol",
        r#"
library Math {
    function double(uint256 x) public pure returns (uint256) {
        return x * 2;
    }
}

contract Counter {
    uint256 public number;

    constructor(uint256 number_) {
        number = Math.double(number_);
    }
}
"#,
    );
    prj.add_source(
        "nested/Counter.sol",
        r#"
library Math {
    function triple(uint256 x) public pure returns (uint256) {
        return x * 3;
    }
}

contract Counter {
    uint256 public number;

    constructor(uint256 number_) {
        number = Math.triple(number_);
    }
}
"#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter as DoubleCounter} from "../src/Counter.sol";
import {Counter as TripleCounter} from "../src/nested/Counter.sol";

contract CounterTest is Test {
    function testNew() public {
        DoubleCounter doubleCounter = new DoubleCounter(21);
        TripleCounter tripleCounter = new TripleCounter(21);
        assertEq(doubleCounter.number(), 42);
        assertEq(tripleCounter.number(), 63);
    }
}
"#,
    );

    cmd.args(["test", "--match-test", "testNew"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testNew() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Counter contract without interface instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//     └── Counter.t.sol
forgetest_init!(preprocess_contract_with_no_interface, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // All files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change Counter implementation to fail single test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 1;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and only one test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 2 != 1] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// Counter contract with interface instantiated in CounterTest
//
// ├── src
// │ ├── Counter.sol
// │ └── interface
// │     └── CounterIf.sol
// └── test
//     └── Counter.t.sol
forgetest_init!(preprocess_contract_with_interface, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "interface/CounterIf.sol",
        r#"
interface CounterIf {
    function number() external returns (uint256);

    function setNumber(uint256 newNumber) external;

    function increment() external;
}
    "#,
    );
    prj.add_source(
        "Counter.sol",
        r#"
import {CounterIf} from "./interface/CounterIf.sol";
contract Counter is CounterIf {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = Counter(address(new Counter()));
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // All 21 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);

    // Change only CounterIf interface.
    prj.add_source(
        "interface/CounterIf.sol",
        r#"
interface CounterIf {
    function anotherNumber() external returns (uint256);

    function number() external returns (uint256);

    function setNumber(uint256 newNumber) external;

    function increment() external;
}
    "#,
    );
    // All 3 files (interface, implementation and test) are compiled.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 3 files with [..]
...

"#]]);

    // Change Counter implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
import {CounterIf} from "./interface/CounterIf.sol";
contract Counter is CounterIf {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);
});

// - Counter contract instantiated in CounterMock
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//     ├── Counter.t.sol
//     └── mock
//         └── CounterMock.sol
forgetest_init!(preprocess_mock_without_inheritance, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    Counter counter = new Counter();

    function setNumber(uint256 newNumber) public {
        counter.setNumber(newNumber);
    }

    function increment() public {
        counter.increment();
    }

    function number() public returns (uint256) {
        return counter.number();
    }
}
    "#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);

    // Change Counter contract implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change CounterMock contract implementation to pass both tests.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    Counter counter = new Counter();

    function setNumber(uint256 newNumber) public {
    }

    function increment() public {
    }

    function number() public returns (uint256) {
        return 1;
    }
}
    "#,
    );
    // Assert that mock and test files are compiled and no test fails.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// - CounterMock contract is Counter contract
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
//    └── mock
//        └── CounterMock.sol
forgetest_init!(preprocess_mock_with_inheritance, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
}
    "#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);

    // Change Counter contract implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256) public virtual {
        number = 12345;
    }

    function increment() public virtual {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert Counter source contract and CounterTest test contract (as it imports mock) are
    // compiled and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 3 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change mock implementation to pass both tests.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
    function setNumber(uint256 newNumber) public override {
        number = newNumber;
    }

    function increment() public override {
        number++;
    }
}
    "#,
    );
    // Assert that CounterMock and CounterTest files are compiled and no test fails.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

forgetest!(preprocess_contract_to_free_function_clears_dependencies, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let dependency =
        "contract Dep { function value() public pure returns (uint256) { return 1; } }";
    prj.add_source("Dep.sol", dependency);
    prj.add_test(
        "Helper.sol",
        r#"
import {Dep} from "../src/Dep.sol";
contract Helper {
    function dependencyCode() internal pure returns (bytes memory) {
        return type(Dep).creationCode;
    }
}
"#,
    );
    prj.add_test(
        "Consumer.t.sol",
        r#"
import {Helper} from "./Helper.sol";
contract ConsumerTest is Helper {
    function test_helper() public pure {
        require(dependencyCode().length > 0);
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_test("Helper.sol", "function helperValue() pure returns (uint256) { return 1; }");
    prj.add_test(
        "Consumer.t.sol",
        r#"
import {helperValue} from "./Helper.sol";
contract ConsumerTest {
    function test_helper() public pure {
        require(helperValue() == 1);
    }
}
"#,
    );
    cmd.assert_success();

    // Neither the contractless helper nor its rebuilt importer still depends on Dep.
    for value in [2, 3] {
        prj.add_source("Dep.sol", &dependency.replace("return 1", &format!("return {value}")));
        cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 1 files with [..]
[..]
Compiler run successful!

Ran 1 test for test/Consumer.t.sol:ConsumerTest
[PASS] test_helper() (gas: [..])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [..]

Ran 1 test suite [..]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
    }
});

// - CounterMock contract is Counter contract
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
//    └── mock
//        └── CounterMock.sol
forgetest_init!(preprocess_mock_to_non_mock, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    let source = r#"
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
    prj.add_source("Counter.sol", source);

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
}
    "#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Change mock implementation to fail tests, no inherit from Counter.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    uint256 public number;
    function setNumber(uint256 newNumber) public {
        number = 1234;
    }

    function increment() public {
        number = 5678;
    }
}
    "#,
    );
    // Assert that CounterMock and CounterTest files are compiled and tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[FAIL: assertion failed: 5678 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 1234 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // The former mock classification must not rebuild importers after a source body-only edit.
    prj.add_source("Counter.sol", &source.replace("number++", "number += 2"));
    prj.forge_command().arg("build").with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 1 files with [..]
[..]
Compiler run successful!

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/12452>
// - CounterMock contract is Counter contract
// - CounterMock declared in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
forgetest_init!(preprocess_mock_declared_in_test_contract, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    function add(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {}

contract CounterTest is Test {
    function test_add() public {
        CounterMock impl = new CounterMock();
        assertEq(impl.add(2, 2), 4);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Change Counter implementation to fail tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    function add(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y + 1;
    }
}
    "#,
    );
    // Assert that Counter and CounterTest files are compiled and tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[FAIL: assertion failed: 5 != 4] test_add() (gas: [..])
...

"#]]);
});

// ├── src
// │ ├── CounterA.sol
// │ ├── CounterB.sol
// │ ├── Counter.sol
// │ └── v1
// │     └── Counter.sol
// └── test
// └── Counter.t.sol
forgetest_init!(preprocess_multiple_contracts_with_constructors, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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
    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 public number;
    address public owner;

    constructor(uint256 _newNumber, address _owner) {
        number = _newNumber;
        owner = _owner;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Contract with constructor args without name.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 public number;

    constructor(uint256) {
        number = 1;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";
import "src/CounterA.sol";
import "src/CounterB.sol";
import {Counter as CounterV1} from "src/v1/Counter.sol";

contract CounterTest is Test {
    function test_Increment_In_Counter() public {
        Counter counter = new Counter();
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_Increment_In_Counter_V1() public {
        CounterV1 counter = new CounterV1(1234);
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_A() public {
        CounterA counter = new CounterA(1234, address(this));
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_A_with_named_args() public {
        CounterA counter = new CounterA({_newNumber: 1234, _owner: address(this)});
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_B() public {
        CounterB counter = new CounterB(1234);
        counter.increment();
        assertEq(counter.number(), 2);
    }
}
    "#,
    );
    // 22 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 24 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[PASS] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[PASS] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change v1/Counter to fail test.
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    );
    // Only v1/Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[PASS] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change CounterA to fail test.
    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 public number;
    address public owner;

    constructor(uint256 _newNumber, address _owner) {
        number = _newNumber;
        owner = _owner;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    );
    // Only CounterA should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change CounterB to fail test.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 public number;

    constructor(uint256) {
        number = 100;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Only CounterB should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[FAIL: assertion failed: 101 != 2] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change Counter to fail test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    );
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12345 != 1] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[FAIL: assertion failed: 101 != 2] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);
});

// Test preprocessing contracts with payable constructor, value and salt named args.
forgetest_init!(flaky_preprocess_contracts_with_payable_constructor_and_salt, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    prj.add_source(
        "CounterWithSalt.sol",
        r#"
contract CounterWithSalt {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";
import {CounterWithSalt} from "src/CounterWithSalt.sol";

contract CounterTest is Test {
    function test_Increment_In_Counter() public {
        Counter counter = Counter(address(new Counter{value: 111}(1)));
        counter.increment();
        assertEq(counter.number(), 112);
    }

    function test_Increment_In_Counter_With_Salt() public {
        CounterWithSalt counter = new CounterWithSalt{value: 111, salt: bytes32("preprocess_counter_with_salt")}(1);
        assertGt(uint160(address(counter)), 0);
        counter.increment();
        assertEq(counter.number(), 112);
    }
}
    "#,
    );

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);

    // Change contract to fail test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value + _number;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);

    // Change contract with salt to fail test too.
    prj.add_source(
        "CounterWithSalt.sol",
        r#"
contract CounterWithSalt {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value + _number;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);
});

// Counter contract with constructor reverts and emitted events.
forgetest_init!(preprocess_contract_with_require_and_emit, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 1, "ctor revert");
        emit CounterCreated(10);
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    function test_assert_constructor_revert() public {
        vm.expectRevert("ctor revert");
        new Counter(1);
    }

    function test_assert_constructor_emit() public {
        vm.expectEmit(true, true, true, true);
        emit Counter.CounterCreated(10);

        new Counter(11);
    }
}
    "#,
    );
    // All 20 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter implementation to revert with different message.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 1, "ctor revert update");
        emit CounterCreated(10);
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and revert test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_assert_constructor_emit() (gas: [..])
[FAIL: Error != expected error: ctor revert update != ctor revert] test_assert_constructor_revert() (gas: [..])
...

"#]]);

    // Change Counter implementation and don't revert.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 0, "ctor revert");
        emit CounterCreated(10);
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and revert test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_assert_constructor_emit() (gas: [..])
[FAIL: next call did not revert as expected] test_assert_constructor_revert() (gas: [..])
...

"#]]);

    // Change Counter implementation to emit different event.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 0, "ctor revert");
        emit CounterCreated(100);
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and emit test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: expected an emit, but no logs were emitted afterwards. you might have mismatched events or not enough events were emitted] test_assert_constructor_emit() (gas: [..])
[FAIL: next call did not revert as expected] test_assert_constructor_revert() (gas: [..])
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10312>
forgetest_init!(preprocess_contract_with_constructor_args_struct, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    struct ConstructorArgs {
        uint256 _number;
    }

    constructor(uint256 no) {
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    function test_assert_constructor_revert() public {
        Counter counter = new Counter(1);
    }
}
    "#,
    );
    // All 20 files should properly compile.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
});

// Test preprocessed contracts with decode internal fns.
forgetest_init!(preprocess_contract_with_decode_internal, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        create_counter(0);
    }

    function test_Increment() public {
        create_counter(0);
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function create_counter(uint256 number) internal {
        counter = new Counter();
        counter.setNumber(number);
    }
}
    "#,
    );

    cmd.args(["test", "--decode-internal", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] test_Increment() ([GAS])
Traces:
  [..] CounterTest::test_Increment()
    ├─ [0] VM::deployCode("src/Counter.sol:Counter")
    │   ├─ [[..]] → new Counter@0xF62849F9A0B5Bf2913b396098F7c7019b51A820a
    │   │   └─ ← [Return] 481 bytes of code
    │   └─ ← [Return] Counter: [0xF62849F9A0B5Bf2913b396098F7c7019b51A820a]
    ├─ [..] Counter::setNumber(0)
    │   └─ ← [Stop]
    ├─ [..] Counter::increment()
    │   └─ ← [Stop]
    ├─ [..] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    ├─ [..] StdAssertions::assertEq(uint256,uint256)(1, 1)
    │   └─ ← 
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10492>
// Preprocess test contracts with try constructor statements.
// Synthetic deployments must respect static execution without changing native try boundaries.
forgetest!(preprocess_static_deployment, |prj, cmd| {
    prj.add_source(
        "Target.sol",
        r#"
contract Empty {
    constructor() payable {}
}
contract Target {
    uint256 public value;
    address public sender;
    constructor(uint256 x) payable { value = x; sender = msg.sender; }
}
"#,
    );
    prj.add_test(
        "StaticDeployment.t.sol",
        r#"
import {Empty, Target} from "../src/Target.sol";

interface Vm {
    function getNonce(address account) external view returns (uint64);
}

contract StaticDeploymentTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function deploy() external returns (Target) { return new Target(7); }
    function deploy2() external returns (Target) { return new Target{salt: bytes32(uint256(1))}(7); }
    function tryDeploy() external { try new Target(7) {} catch {} }
    function tryDeploy2() external { try new Target{salt: bytes32(uint256(1))}(7) {} catch {} }
    function creationCode() external view returns (bytes memory) { return type(Target).creationCode; }

    function checkStatic(bytes memory data) internal {
        uint64 nonce = vm.getNonce(address(this));
        uint256 balance = address(this).balance;
        (bool ok, bytes memory result) = address(this).staticcall(data);
        require(!ok, "static deployment succeeded");
        require(result.length == 0, "unexpected revert data");
        require(vm.getNonce(address(this)) == nonce, "nonce changed");
        require(address(this).balance == balance, "balance changed");
    }

    function test_static_create() public { checkStatic(abi.encodeCall(this.deploy, ())); }
    function test_static_create2() public { checkStatic(abi.encodeCall(this.deploy2, ())); }
    function test_static_try_create() public { checkStatic(abi.encodeCall(this.tryDeploy, ())); }
    function test_static_try_create2() public { checkStatic(abi.encodeCall(this.tryDeploy2, ())); }

    function test_regular_deployment() public {
        Target a = this.deploy();
        Target b = this.deploy2();
        require(a.value() == 7 && b.value() == 7);
        require(a.sender() == address(this) && b.sender() == address(this));
    }

    function test_static_creation_code() public {
        (bool ok, bytes memory result) = address(this).staticcall(abi.encodeCall(this.creationCode, ()));
        require(ok && abi.decode(result, (bytes)).length > 0);
    }

    function test_manual_deploy_code_static() public {
        string memory empty = "src/Target.sol:Empty";
        string memory target = "src/Target.sol:Target";
        bytes memory args = abi.encode(uint256(7));
        bytes32 salt = bytes32(uint256(1));
        bytes[] memory calls = new bytes[](8);
        calls[0] = abi.encodeWithSignature("deployCode(string)", empty);
        calls[1] = abi.encodeWithSignature("deployCode(string,bytes)", target, args);
        calls[2] = abi.encodeWithSignature("deployCode(string,uint256)", empty, 1);
        calls[3] = abi.encodeWithSignature("deployCode(string,bytes,uint256)", target, args, 1);
        calls[4] = abi.encodeWithSignature("deployCode(string,bytes32)", empty, salt);
        calls[5] = abi.encodeWithSignature("deployCode(string,bytes,bytes32)", target, args, salt);
        calls[6] = abi.encodeWithSignature("deployCode(string,uint256,bytes32)", empty, 1, salt);
        calls[7] = abi.encodeWithSignature("deployCode(string,bytes,uint256,bytes32)", target, args, 1, salt);
        uint64 nonce = vm.getNonce(address(this));
        for (uint256 i; i < calls.length; ++i) {
            (bool ok, bytes memory result) = address(vm).staticcall(calls[i]);
            require(!ok && result.length == 0, "static deployCode succeeded");
        }
        require(vm.getNonce(address(this)) == nonce, "nonce changed");
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        for force in [true, false] {
            cmd.forge_fuse().arg("test");
            if force {
                cmd.arg("--force");
            }
            cmd.assert_success().stdout_eq(str![[r#"
...
Ran 7 tests for test/StaticDeployment.t.sol:StaticDeploymentTest
[PASS] test_manual_deploy_code_static() ([GAS])
[PASS] test_regular_deployment() ([GAS])
[PASS] test_static_create() ([GAS])
[PASS] test_static_create2() ([GAS])
[PASS] test_static_creation_code() ([GAS])
[PASS] test_static_try_create() ([GAS])
[PASS] test_static_try_create2() ([GAS])
Suite result: ok. 7 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
        }
    }
});

forgetest_init!(preprocess_contract_with_try_ctor_stmt, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 number;
}
    "#,
    );
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 0, "ctor failure");
        number = a;
    }
}
    "#,
    );
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 0, "ctor failure");
        number = a;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterA} from "../src/CounterA.sol";
import {CounterB} from "../src/CounterB.sol";
import {CounterC} from "../src/CounterC.sol";

contract CounterTest is Test {
    function test_try_counterA_creation() public {
        try new CounterA() {} catch {
            revert();
        }
    }

    function test_try_counterB_creation() public {
        try new CounterB(1) {} catch {
            revert();
        }
    }

    function test_try_counterB_creation_with_salt() public {
        try new CounterB{value: 111, salt: bytes32("preprocess_counter_with_salt")}(1) {} catch {
            revert();
        }
    }

    function test_try_counterC_creation() public {
        try new CounterC(2) {
            new CounterC(1);
        } catch {
            revert();
        }
    }
}
    "#,
    );
    // All 23 files should properly compile, tests pass.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[PASS] test_try_counterB_creation() (gas: [..])
[PASS] test_try_counterB_creation_with_salt() (gas: [..])
[PASS] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterB to fail test.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 11, "ctor failure");
        number = a;
    }
}
    "#,
    );
    // CounterB and its native try-deployment consumer should compile.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[PASS] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterC to fail test in try statement.
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 1, "ctor failure");
        number = a;
    }
}
    "#,
    );
    // CounterC and its native try-deployment consumer should compile.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[FAIL: ctor failure] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterC to fail test in try statement.
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 2, "ctor failure");
        number = a;
    }
}
    "#,
    );
    // CounterC and its native try-deployment consumer should compile and revert.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterC_creation() (gas: [..])
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/11978>
// Preprocess test contracts when active prank.
forgetest_init!(preprocess_contract_with_active_prank, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;
    address public deployer;
    address public origin;

    constructor() {
        deployer = msg.sender;
        origin = tx.origin;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    function test_deployer() public {
        address deployer = makeAddr("deployer");
        address origin = makeAddr("origin");
        vm.startPrank(deployer, origin);
        Counter first = new Counter{salt: 0}();
        Counter second = new Counter{salt: bytes32(uint256(1))}();
        assertEq(first.deployer(), deployer);
        assertEq(first.origin(), origin);
        assertEq(second.deployer(), deployer);
        assertEq(second.origin(), origin);
    }

    function test_consecutive_single_call_pranks() public {
        address firstDeployer = makeAddr("firstDeployer");
        address firstOrigin = makeAddr("firstOrigin");
        vm.prank(firstDeployer, firstOrigin);
        Counter first = new Counter();

        address secondDeployer = makeAddr("secondDeployer");
        address secondOrigin = makeAddr("secondOrigin");
        vm.prank(secondDeployer, secondOrigin);
        Counter second = new Counter();

        assertEq(first.deployer(), firstDeployer);
        assertEq(first.origin(), firstOrigin);
        assertEq(second.deployer(), secondDeployer);
        assertEq(second.origin(), secondOrigin);

        Counter unpranked = new Counter();
        assertEq(unpranked.deployer(), address(this));
        assertEq(unpranked.origin(), tx.origin);
    }
}
    "#,
    );
    // Test should pass.
    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] test_consecutive_single_call_pranks() ([GAS])
[PASS] test_deployer() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

// Preprocess test contracts with try constructor statements that bind return type.
forgetest_init!(preprocess_contract_with_try_ctor_stmt_and_returns, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 0, "ctor failure");
        number = a;
    }
}
        "#,
    );
    prj.add_test(
        "CounterReturns.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterReturnsTest is Test {
    function test_try_counter_creation_returns_custom_type() public {
        try new Counter(1) returns (Counter c) {
            c;
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("ctor failure"));
        }
    }
}
        "#,
    );

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...
[PASS] test_try_counter_creation_returns_custom_type() (gas: [..])
...

"#]]);

    // The typed deployment remains native so its constructor revert reaches the catch clause.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 number;
    constructor(uint256 a) payable {
        require(a == 0, "ctor failure");
        number = a;
    }
}
        "#,
    );
    cmd.assert_success();
});

forgetest!(preprocess_typed_try_new_preserves_catches, |prj, cmd| {
    let targets = r#"
error ConstructorError(uint256 value);

contract RevertString {
    constructor() { revert("first reason"); }
}

contract CustomError {
    constructor() { revert ConstructorError(7); }
}

contract Panic {
    constructor() { assert(false); }
}

contract EmptyRevert {
    constructor() { assembly { revert(0, 0) } }
}
"#;
    prj.add_source("Targets.sol", targets);
    prj.add_test(
        "TypedTry.t.sol",
        r#"
import * as Targets from "../src/Targets.sol";

contract TypedTryTest {
    bool public constructorCaught;

    constructor() {
        try new Targets.EmptyRevert() returns (Targets.EmptyRevert) {
            revert("constructor deployment succeeded");
        } catch (bytes memory reason) {
            constructorCaught = reason.length == 0;
        }
    }

    function test_constructor_context() public view {
        require(constructorCaught, "constructor catch missed");
    }

    function test_error_string() public {
        try new Targets.RevertString() returns (Targets.RevertString) {
            revert("deployment succeeded");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("first reason"), "changed reason");
        }
    }

    function test_custom_error() public {
        try new Targets.CustomError() returns (Targets.CustomError) {
            revert("deployment succeeded");
        } catch (bytes memory reason) {
            require(bytes4(reason) == Targets.ConstructorError.selector, "wrong custom error");
        }
    }

    function test_panic() public {
        try new Targets.Panic() returns (Targets.Panic) {
            revert("deployment succeeded");
        } catch Panic(uint256 code) {
            require(code == 1, "wrong panic");
        }
    }

    function test_empty_revert() public {
        try new Targets.EmptyRevert() returns (Targets.EmptyRevert) {
            revert("deployment succeeded");
        } catch (bytes memory reason) {
            require(reason.length == 0, "non-empty revert");
        }
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Targets.sol", targets);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Targets.sol", &targets.replace("first reason", "second reason"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 5 tests for test/TypedTry.t.sol:TypedTryTest
[PASS] test_constructor_context() ([GAS])
[PASS] test_custom_error() ([GAS])
[PASS] test_empty_revert() ([GAS])
[FAIL: changed reason] test_error_string() ([GAS])
[PASS] test_panic() ([GAS])
Suite result: FAILED. 4 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

// Test that `type(Contract).creationCode` can be used in view functions.
// https://github.com/foundry-rs/foundry/issues/13086
forgetest_init!(preprocess_creation_code_in_view_function, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target {
    uint256 public immutable value;
    constructor(uint256 _value) { value = _value; }
}
        "#,
    );

    prj.add_test(
        "Target.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Target} from "../src/Target.sol";

contract TargetTest is Test {
    function computeAddress(address factory, uint256 salt, uint256 value) internal view returns (address) {
        bytes32 hash = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                factory,
                salt,
                keccak256(abi.encodePacked(type(Target).creationCode, abi.encode(value)))
            )
        );
        return address(uint160(uint256(hash)));
    }

    function testComputeAddress() public view {
        computeAddress(address(this), 1, 100);
    }
}
        "#,
    );

    cmd.args(["build"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/16487>
forgetest_init!(preprocess_custom_layout_contract, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.solc = Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 35)));
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target layout at erc7201("test.Target") {
    uint256 public value;

    constructor(uint256 value_) {
        value = value_;
    }
}
        "#,
    );

    prj.add_test(
        "Target.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Target} from "../src/Target.sol";

contract TargetTest is Test {
    function testDirectNew() public {
        Target target = new Target(42);
        assertEq(target.value(), 42);
    }

    function targetCreationCode() public view returns (bytes memory) {
        return type(Target).creationCode;
    }
}
        "#,
    );

    cmd.args(["test"]).assert_success();
});

// Test that `type(Contract).creationCode` keeps native pure semantics when dynamic linking is
// enabled.
forgetest_init!(preprocess_creation_code_in_pure_function, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target {
    uint256 public immutable value;
    constructor(uint256 _value) { value = _value; }
}
        "#,
    );

    prj.add_test(
        "Target.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Target} from "../src/Target.sol";

contract TargetTest is Test {
    function computeAddress(address factory, uint256 salt, uint256 value) internal pure returns (address) {
        bytes32 hash = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                factory,
                salt,
                keccak256(abi.encodePacked(type(Target).creationCode, abi.encode(value)))
            )
        );
        return address(uint160(uint256(hash)));
    }

    function testComputeAddress() public pure {
        computeAddress(address(0xBEEF), 1, 100);
    }
}
        "#,
    );

    cmd.args(["build"]).assert_success();
});

// Constant initializers must stay native and retain their bytecode dependencies.
forgetest!(preprocess_creation_code_in_constant_initializer, |prj, cmd| {
    let target = r#"
contract Target {
    function value() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_test(
        "ConstantCode.t.sol",
        r#"
import {Target} from "../src/Target.sol";

contract ConstantCodeTest {
    bytes constant CODE = type(Target).creationCode;
    bytes32 constant CODE_HASH = keccak256(type(Target).creationCode);
    bytes32 immutable initialHash = keccak256(type(Target).creationCode);

    function test_constant_code() public {
        require(CODE_HASH == initialHash, "stale code hash");
        bytes memory code = CODE;
        address deployed;
        assembly { deployed := create(0, add(code, 32), mload(code)) }
        require(Target(deployed).value() == 111, "changed value");
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Target.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped
...
"#]]);

        prj.add_source("Target.sol", &target.replace("111", "222"));
        for force in [false, true] {
            cmd.forge_fuse().arg("test");
            if force {
                cmd.arg("--force");
            }
            cmd.assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/ConstantCode.t.sol:ConstantCodeTest
[FAIL: changed value] test_constant_code() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
        }

        prj.add_source("Target.sol", target);
        cmd.forge_fuse().arg("test").assert_success();
    }
});

// Test that `type(Contract).creationCode` keeps native pure semantics when it is used in a
// modifier body that is applied to a pure function.
forgetest_init!(preprocess_creation_code_in_modifier_used_by_pure_function, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target {}
        "#,
    );

    prj.add_test(
        "ModifierCreationCode.t.sol",
        r#"
import {Target} from "../src/Target.sol";

contract ModifierCreationCodeTest {
    modifier usesCreationCode() {
        bytes memory code = type(Target).creationCode;
        code;
        _;
    }

    function testModifierCreationCode() public pure usesCreationCode {}
}
        "#,
    );

    cmd.args(["build"]).assert_success();
});

// Nested call options are copied verbatim and must retain native dependency edges.
forgetest!(preprocess_nested_deployment_options, |prj, cmd| {
    let other = r#"
contract Other {
    function value() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Other.sol", other);
    prj.add_source(
        "Target.sol",
        r#"
contract Empty { constructor() payable {} }
contract Target {
    uint256 public immutable value;
    constructor(uint256 value_) payable { value = value_; }
}
"#,
    );
    prj.add_test(
        "Options.t.sol",
        r#"
import {Other} from "../src/Other.sol";
import {Empty, Target} from "../src/Target.sol";
contract OptionsTest {
    function expected() internal pure returns (bytes32) {
        return keccak256(type(Empty).creationCode);
    }
    function test_salt() public {
        Empty target = new Empty{salt: bytes32(new Other().value())}();
        address predicted = address(uint160(uint256(keccak256(abi.encodePacked(
            bytes1(0xff), address(this), bytes32(uint256(111)), expected()
        )))));
        require(address(target) == predicted, "changed salt");
    }
    function test_value() public {
        Empty target = new Empty{value: new Other().value()}();
        require(address(target).balance == 111, "changed value");
    }
    function test_arguments() public {
        Target target = new Target{value: new Other().value()}(new Other().value());
        require(address(target).balance == 111 && target.value() == 111, "changed arguments");
    }
    function test_try() public {
        try new Target{value: new Other().value()}(42) returns (Target target) {
            require(address(target).balance == 111, "changed try value");
        } catch { revert("deployment failed"); }
    }
}
"#,
    );
    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Other.sol", other);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Other.sol", &other.replace("return 111", "return 222"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 4 tests for test/Options.t.sol:OptionsTest
[FAIL: changed arguments] test_arguments() ([GAS])
[FAIL: changed salt] test_salt() ([GAS])
[FAIL: changed try value] test_try() ([GAS])
[FAIL: changed value] test_value() ([GAS])
Suite result: FAILED. 0 passed; 4 failed; 0 skipped; [ELAPSED]
...
"#]]);
        cmd.forge_fuse().args(["test", "--force"]).assert_failure();
    }
});

// Constructor helper fields use type spans, independent of parameter data-location spelling.
forgetest!(preprocess_constructor_parameter_types, |prj, cmd| {
    let target = r#"
contract Target {
    uint256 public value;
    constructor(
        bytes
        memory
        a,
        string/* before */memory/* after */b,
        uint256[]	memory	c,
        bytes memory,
        function(bytes memory) external returns (bytes memory) callback
    ) {
        require(a.length == 1 && bytes(b).length == 2 && c.length == 3);
        require(callback(a).length == 1);
        value = 111;
    }
}
contract Named {
    uint256 public value;
    constructor(bytes/* before */memory/* after */data) {
        require(data.length == 1);
        value = 111;
    }
}
"#;
    prj.add_source("Target.sol", target);
    prj.add_test(
        "Parameters.t.sol",
        r#"
import {Target, Named} from "../src/Target.sol";
contract ParametersTest {
    function echo(bytes memory data) external pure returns (bytes memory) { return data; }
    function test_positional() public {
        Target target = new Target(hex"01", "ab", new uint256[](3), hex"", this.echo);
        require(target.value() == 111, "changed positional");
    }
    function test_named() public {
        Named target = new Named({data: hex"01"});
        require(target.value() == 111, "changed named");
    }
}
"#,
    );
    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Target.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Target.sol", &target.replace("value = 111", "value = 222"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/Parameters.t.sol:ParametersTest
[FAIL: changed named] test_named() ([GAS])
[FAIL: changed positional] test_positional() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...
"#]]);
        cmd.forge_fuse().args(["test", "--force"]).assert_failure();
    }
});

forgetest!(preprocess_private_constructor_array_dimensions, |prj, cmd| {
    let targets = r#"
contract Nested {
    uint256 private constant N = 2;
    constructor(uint256[N][3] memory xs) { require(xs[2][1] == 7); }
    function value() external pure returns (uint256) { return 111; }
}
contract Expression {
    uint256 private constant N = 2;
    constructor(uint256[N + 1] memory xs) { require(xs[2] == 7); }
    function value() external pure returns (uint256) { return 111; }
}
contract Unnamed {
    uint256 private constant N = 2;
    constructor(uint256[N] memory) {}
    function value() external pure returns (uint256) { return 111; }
}
contract Literal {
    constructor(uint256[2] memory) {}
    function value() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_test(
        "PrivateDimensions.t.sol",
        r#"
import {Expression, Literal, Nested, Unnamed} from "../src/Targets.sol";

contract PrivateDimensionsTest {
    function test_nested() public {
        uint256[2][3] memory xs;
        xs[2][1] = 7;
        require(new Nested(xs).value() == 111, "changed value");
    }
    function test_expression() public {
        uint256[3] memory xs;
        xs[2] = 7;
        require(new Expression(xs).value() == 111, "changed value");
    }
    function test_unnamed() public {
        require(new Unnamed([uint256(1), 2]).value() == 111, "changed value");
    }
    function test_literal() public {
        require(new Literal([uint256(1), 2]).value() == 111, "changed value");
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Targets.sol", targets);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();

        prj.add_source("Targets.sol", &targets.replace("111", "222"));
        for force in [false, true] {
            cmd.forge_fuse().arg("test");
            if force {
                cmd.arg("--force");
            }
            cmd.assert_failure().stdout_eq(str![[r#"
...
Ran 4 tests for test/PrivateDimensions.t.sol:PrivateDimensionsTest
[FAIL: changed value] test_expression() ([GAS])
[FAIL: changed value] test_literal() ([GAS])
[FAIL: changed value] test_nested() ([GAS])
[FAIL: changed value] test_unnamed() ([GAS])
Suite result: FAILED. 0 passed; 4 failed; 0 skipped; [ELAPSED]
...
"#]]);
        }

        prj.add_source("Targets.sol", targets);
        cmd.forge_fuse().arg("test").assert_success();
    }

    cmd.forge_fuse()
        .args(["test", "--match-test", "test_literal", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Traces:
...
    ├─ [0] VM::deployCode("src/Targets.sol:Literal", 0x[..])
...
"#]]);
});

forgetest!(preprocess_generated_interface_name_collision, |prj, cmd| {
    let target = r#"
contract Target {
    function value() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Target.sol", target);
    prj.add_source(
        "ImportedNames.sol",
        "interface VmContractHelper2 {} interface VmContractHelper2_ {}",
    );
    prj.add_test(
        "Collision.t.sol",
        r#"
import {Target} from "../src/Target.sol";
import "../src/ImportedNames.sol";

contract CollisionTest {
    function test_value() public {
        require(new Target().value() == 111, "changed value");
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Target.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Target.sol", &target.replace("return 111", "return 222"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Collision.t.sol:CollisionTest
[FAIL: changed value] test_value() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

forgetest!(preprocess_generated_constructor_helper_name_collision, |prj, cmd| {
    let base = r#"
contract Base {
    struct FoundryPpConstructorArgs { uint256 unused; }
    function encodeArgs0() public pure returns (uint256) { return 0; }
}
"#;
    let target = r#"
import {Base} from "./Base.sol";
contract Target is Base {
    uint256 public value;
    constructor(uint256, uint256 foundry_pp_ctor_arg1) {
        value = foundry_pp_ctor_arg1 + 110;
    }
}
contract DeployHelper0 {}
function encodeArgs0() pure returns (uint256) { return 0; }
"#;
    prj.add_source("Base.sol", base);
    prj.add_source("Target.sol", target);
    prj.add_test(
        "Collision.t.sol",
        r#"
import {Target} from "../src/Target.sol";

contract DeployHelper0_ {}
function encodeArgs0_() pure returns (uint256) { return 0; }

contract CollisionTest {
    function test_value() public {
        require(new Target(0, 1).value() == 111, "changed value");
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Base.sol", base);
        prj.add_source("Target.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Target.sol", &target.replace("+ 110", "+ 220"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Collision.t.sol:CollisionTest
[FAIL: changed value] test_value() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

forgetest!(preprocess_try_adapter_name_collision, |prj, cmd| {
    let target = r#"
contract Target {
    function value() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Target.sol", target);
    prj.add_test(
        "Collision.t.sol",
        r#"
import {Target} from "../src/Target.sol";

contract CollisionTest {
    function addressToTarget1(address target) public pure returns (Target) {
        return Target(target);
    }

    function addressToTarget1_(address target) public pure returns (Target) {
        return Target(target);
    }

    function test_value() public {
        try new Target() returns (Target target) {
            require(target.value() == 111, "changed value");
        } catch {
            revert("deployment failed");
        }
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Target.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Target.sol", &target.replace("return 111", "return 222"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Collision.t.sol:CollisionTest
[FAIL: changed value] test_value() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

forgetest!(preprocess_payable_try_adapter, |prj, cmd| {
    let target = r#"
contract Target {
    uint256 public value;
    constructor(uint256 offset) payable { value = offset + 111; }
    receive() external payable {}
}
"#;
    prj.add_source("Target.sol", target);
    prj.add_test(
        "Payable.t.sol",
        r#"
import {Target} from "../src/Target.sol";

contract PayableTest {
    function test_value() public {
        try new Target(0) returns (Target target) {
            require(target.value() == 111, "changed value");
        } catch {
            revert("deployment failed");
        }
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Target.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source("Target.sol", &target.replace("+ 111", "+ 222"));
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Payable.t.sol:PayableTest
[FAIL: changed value] test_value() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

// Windows filenames cannot contain double quotes.
#[cfg(unix)]
forgetest!(preprocess_generated_path_string_escaping, |prj, cmd| {
    let target = r#"
contract Zero {
    function value() external pure returns (uint256) { return 111; }
}
contract Args {
    uint256 public value;
    constructor(uint256 offset) { value = offset + 111; }
}
"#;
    prj.add_source("Quoted\"Path.sol", target);
    prj.add_test(
        "Quoted.t.sol",
        r#"
import {Zero, Args} from "../src/Quoted\"Path.sol";

contract QuotedTest {
    function test_zero() public {
        require(new Zero().value() == 111, "changed zero");
    }

    function test_args() public {
        require(new Args(0).value() == 111, "changed args");
    }
}
"#,
    );

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Quoted\"Path.sol", target);
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source(
            "Quoted\"Path.sol",
            &target.replace("+ 111", "+ 222").replace("return 111", "return 222"),
        );
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/Quoted.t.sol:QuotedTest
[FAIL: changed args] test_args() ([GAS])
[FAIL: changed zero] test_zero() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

// Windows directory names cannot contain colons.
#[cfg(unix)]
forgetest!(preprocess_colon_in_artifact_path, |prj, cmd| {
    let target = r#"
contract Zero {
    function value() external pure returns (uint256) { return 111; }
}
contract Args {
    uint256 public value;
    constructor(uint256 offset) { value = offset + 111; }
}
"#;
    prj.add_source("Colon:Path.sol", target);
    prj.add_source(
        "Parsed:Wrong.sol",
        "contract Parsed { function value() external pure returns (uint256) { return 111; } }",
    );
    prj.add_test(
        "Colon.t.sol",
        r#"
import {Zero, Args} from "../src/Colon:Path.sol";
import {Parsed} from "../src/Parsed:Wrong.sol";

contract ColonTest {
    function test_zero() public {
        require(new Zero().value() == 111, "changed zero");
    }

    function test_args() public {
        require(new Args(0).value() == 111, "changed args");
    }

    function test_parseable_identifier() public {
        require(new Parsed().value() == 111, "changed parsed");
    }

    function test_running_profile_bytecode() public {
        require(
            keccak256(address(new Zero()).code) == keccak256(type(Zero).runtimeCode),
            "wrong profile bytecode"
        );
    }
}
"#,
    );
    prj.update_config(|config| {
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "optimized".to_owned(),
            via_ir: None,
            evm_version: None,
            optimizer: Some(true),
            optimizer_runs: Some(10_000),
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "test/Colon.t.sol".parse().unwrap(),
            version: None,
            via_ir: None,
            bytecode_hash: None,
            min_optimizer_runs: Some(10_000),
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: None,
        }];
    });

    for dynamic_test_linking in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic_test_linking);
        prj.add_source("Colon:Path.sol", target);
        prj.add_source(
            "Parsed:Wrong.sol",
            "contract Parsed { function value() external pure returns (uint256) { return 111; } }",
        );
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source(
            "Colon:Path.sol",
            &target.replace("+ 111", "+ 222").replace("return 111", "return 222"),
        );
        prj.add_source(
            "Parsed:Wrong.sol",
            "contract Parsed { function value() external pure returns (uint256) { return 222; } }",
        );
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 4 tests for test/Colon.t.sol:ColonTest
[FAIL: changed args] test_args() ([GAS])
[FAIL: changed parsed] test_parseable_identifier() ([GAS])
[PASS] test_running_profile_bytecode() ([GAS])
[FAIL: changed zero] test_zero() ([GAS])
Suite result: FAILED. 1 passed; 3 failed; 0 skipped; [ELAPSED]
...
"#]]);
    }
});

forgetest!(preprocess_create_isolation_boundary, |prj, cmd| {
    prj.add_source(
        "Target.sol",
        r#"
interface Callback {
    function read() external view returns (uint256);
    function write() external;
}
contract Target {
    uint256 public observed;
    constructor() {
        observed = Callback(msg.sender).read();
        Callback(msg.sender).write();
    }
}
"#,
    );
    for isolate in [false, true] {
        prj.update_config(|config| {
            config.isolate = isolate;
            config.evm_version = EvmVersion::Cancun;
        });
        prj.add_test(
            "Isolation.t.sol",
            &format!(
                r#"
import {{Target}} from '../src/Target.sol';
contract IsolationTest {{
    function read() external view returns (uint256 n) {{ assembly {{ n := tload(0) }} }}
    function write() external {{ assembly {{ tstore(0, 22) }} }}
    function test_create() public {{
        assembly {{ tstore(0, 11) }}
        Target target = new Target();
        require(target.observed() == {}, "constructor boundary");
        uint256 n;
        assembly {{ n := tload(0) }}
        require(n == {}, "outer boundary");
    }}
    function test_create2() public {{
        assembly {{ tstore(0, 11) }}
        Target target = new Target{{salt: bytes32(uint256(7))}}();
        require(target.observed() == 11, "salted constructor boundary");
        uint256 n;
        assembly {{ n := tload(0) }}
        require(n == 22, "salted outer boundary");
    }}
}}
"#,
                if isolate { 0 } else { 11 },
                if isolate { 11 } else { 22 },
            ),
        );
        for dynamic in [false, true] {
            prj.update_config(|config| config.dynamic_test_linking = dynamic);
            cmd.forge_fuse().args(["test", "--force"]).assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/Isolation.t.sol:IsolationTest
[PASS] test_create() ([GAS])
[PASS] test_create2() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
        }
    }
    // Confirm the isolation regression exercised a rewritten CREATE and balanced nested traces.
    cmd.forge_fuse()
        .args(["test", "--match-test", r"^test_create\(", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/Isolation.t.sol:IsolationTest
[PASS] test_create() ([GAS])
Traces:
  [[..]] IsolationTest::test_create()
    ├─ [0] VM::deployCode("src/Target.sol:Target")
    │   ├─ [[..]] → new Target@[..]
    │   │   ├─ [[..]] IsolationTest::read() [staticcall]
    │   │   │   └─ ← [Return] 0
    │   │   ├─ [[..]] IsolationTest::write()
    │   │   │   └─ ← [Stop]
    │   │   └─ ← [Return] [..] bytes of code
    │   └─ ← [Return] Target: [..]
    ├─ [[..]] Target::observed() [staticcall]
    │   └─ ← [Return] 0
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(preprocess_constructor_validation, |prj, cmd| {
    // These constraints would disappear with the original new-expression. Solc must reject
    // exactly the same source in both modes, including implicit and explicit constructors.
    for (target, expression) in [
        ("contract Target {}", "new Target(7)"),
        ("contract Target {}", "new Target({oops: 7})"),
        ("contract Target { constructor() {} }", "new Target(7)"),
        ("contract Target {}", "new Target{gas: 100000}()"),
        ("contract Target {}", "new Target{value: 0}()"),
        ("contract Target { constructor(uint256 x) {} }", "new Target{gas: 100000}(7)"),
        ("contract Target { constructor(uint256 x) {} }", "new Target{value: 0}(7)"),
        ("contract Target { constructor(uint256 x) {} }", "new Target({oops: 7})"),
        ("contract Target { constructor(uint256 x, uint256 y) {} }", "new Target({x: 7, x: 8})"),
        ("library Target {}", "new Target()"),
        ("abstract contract Target { function f() external virtual; }", "new Target()"),
    ] {
        prj.add_source("Target.sol", target);
        prj.add_test(
            "Invalid.t.sol",
            &format!(
                "import '../src/Target.sol'; contract InvalidTest {{ function test_invalid() public {{ {expression}; }} }}"
            ),
        );
        prj.update_config(|config| config.dynamic_test_linking = false);
        let native = cmd
            .forge_fuse()
            .args(["build", "--force"])
            .assert_failure()
            .get_output()
            .stderr
            .clone();
        prj.update_config(|config| config.dynamic_test_linking = true);
        cmd.forge_fuse().args(["build", "--force"]).assert_failure().stderr_eq(native);
    }
});

forgetest!(preprocess_constructor_abi_coder_validation, |prj, cmd| {
    // Encoding in the generated helper must not bypass the caller's ABI-coder restrictions.
    for (parameters, arguments) in
        [("S memory s", "Target.S(7)"), ("uint256[][] memory xs", "new uint256[][](0)")]
    {
        prj.add_source(
            "Target.sol",
            &format!(
                "pragma abicoder v2; contract Target {{ struct S {{ uint256 n; }} constructor({parameters}) {{}} }}"
            ),
        );
        for pragma in
            ["pragma abicoder v1;", "pragma abicoder v2;", "pragma experimental ABIEncoderV2;"]
        {
            prj.add_test(
                "Coder.t.sol",
                &format!(
                    "{pragma} import '../src/Target.sol'; contract CoderTest {{ function test_construct() public {{ new Target({arguments}); }} }}"
                ),
            );
            prj.update_config(|config| config.dynamic_test_linking = false);
            if pragma == "pragma abicoder v1;" {
                let native = cmd
                    .forge_fuse()
                    .args(["build", "--force"])
                    .assert_failure()
                    .get_output()
                    .stderr
                    .clone();
                prj.update_config(|config| config.dynamic_test_linking = true);
                cmd.forge_fuse().args(["build", "--force"]).assert_failure().stderr_eq(native);
            } else {
                cmd.forge_fuse().args(["test", "--force"]).assert_success();
                prj.update_config(|config| config.dynamic_test_linking = true);
                cmd.forge_fuse().args(["test", "--force"]).assert_success();
            }
        }
    }
});

forgetest!(preprocess_constructor_abi_v1_native_cache, |prj, cmd| {
    prj.add_test(
        "Coder.t.sol",
        r#"
pragma abicoder v1;
import "../src/Target.sol";
contract CoderTest {
    function test_construct() public {
        require(new Target(7).value() == 7, "stale constructor");
    }
}
"#,
    );
    for dynamic in [false, true] {
        prj.update_config(|config| config.dynamic_test_linking = dynamic);
        prj.add_source(
            "Target.sol",
            "contract Target { uint256 public value; constructor(uint256 n) { value = n; } }",
        );
        cmd.forge_fuse().args(["test", "--force"]).assert_success();
        cmd.forge_fuse().arg("test").assert_success();
        prj.add_source(
            "Target.sol",
            "contract Target { uint256 public value; constructor(uint256 n) { value = n + 1; } }",
        );
        for force in [false, true] {
            cmd.forge_fuse().arg("test");
            if force {
                cmd.arg("--force");
            }
            cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale constructor] test_construct() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
        }
    }
});

forgetest!(preprocess_constructor_evm_version_validation, |prj, cmd| {
    // The compilation target controls CREATE2 validation, independently of runtime settings.
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 28)));
    });
    prj.add_source("Target.sol", "pragma solidity ^0.8.0; contract Target {}");
    for (evm_version, salt, valid) in [
        (EvmVersion::Byzantium, "{salt: bytes32(0)}", false),
        (EvmVersion::Byzantium, "", true),
        (EvmVersion::Constantinople, "{salt: bytes32(0)}", true),
    ] {
        prj.add_test(
            "Version.t.sol",
            &format!("pragma solidity ^0.8.0; import '../src/Target.sol'; contract VersionTest {{ function test_construct() public {{ new Target{salt}(); }} }}"),
        );
        prj.update_config(|config| {
            config.evm_version = evm_version;
            config.dynamic_test_linking = false;
        });
        if valid {
            cmd.forge_fuse().args(["test", "--force"]).assert_success();
            prj.update_config(|config| config.dynamic_test_linking = true);
            cmd.forge_fuse().args(["test", "--force"]).assert_success();
        } else {
            let native = cmd
                .forge_fuse()
                .args(["build", "--force"])
                .assert_failure()
                .get_output()
                .stderr
                .clone();
            prj.update_config(|config| config.dynamic_test_linking = true);
            cmd.forge_fuse().args(["build", "--force"]).assert_failure().stderr_eq(native);
        }
    }
});

forgetest!(preprocess_return_data_observation, |prj, cmd| {
    prj.add_source("Target.sol", "contract Target {}");
    // Each case has its own contract and helper source, so another observer cannot mask a
    // discovery failure. Abstract bases keep inherited tests in the derived suite only.
    for (case, helper, declarations) in [
        (
            "create",
            "",
            r#"
contract ReturnDataTest {
    function test_return_data() public {
        new Target();
        uint256 n;
        assembly { n := returndatasize() }
        require(n == 0, "create buffer");
    }
}"#,
        ),
        (
            "namespace_create2",
            "function size() view returns (uint256 n) { assembly { n := returndatasize() } }",
            r#"
import * as Observe from '../src/Observe.sol';
contract ReturnDataTest {
    function test_return_data() public {
        new Target{salt: bytes32(uint256(7))}();
        require(Observe.size() == 0, "create2 buffer");
    }
}"#,
        ),
        (
            "inherited_producer",
            "function size() view returns (uint256 n) { assembly { n := returndatasize() } }",
            r#"
import * as Observe from '../src/Observe.sol';
abstract contract Base { function make() internal { new Target(); } }
contract ReturnDataTest is Base {
    function test_return_data() public {
        make();
        require(Observe.size() == 0, "inherited producer buffer");
    }
}"#,
        ),
        (
            "inherited_observer",
            "",
            r#"
abstract contract Base {
    function make() internal virtual;
    function test_return_data() public {
        make();
        uint256 n;
        assembly { n := returndatasize() }
        require(n == 0, "inherited observer buffer");
    }
}
contract ReturnDataTest is Base {
    function make() internal override { new Target(); }
}"#,
        ),
        (
            "using_initializer",
            "library Reader { function size(uint256) internal view returns (uint256 n) { assembly { n := returndatasize() } } }",
            r#"
import {Reader} from '../src/Observe.sol';
contract ReturnDataTest {
    using Reader for uint256;
    Target target = new Target();
    uint256 observed = uint256(0).size();
    function test_return_data() public view {
        require(observed == 0, "initializer buffer");
    }
}"#,
        ),
        (
            "using_function",
            "function size(uint256) view returns (uint256 n) { assembly { n := returndatasize() } }",
            r#"
import {size} from '../src/Observe.sol';
contract ReturnDataTest {
    using {size} for uint256;
    function test_return_data() public {
        new Target();
        require(uint256(0).size() == 0, "using function buffer");
    }
}"#,
        ),
        (
            "aliased_using",
            "function size(uint256) view returns (uint256 n) { assembly { n := returndatasize() } }",
            r#"
import {size as readSize} from '../src/Observe.sol';
contract ReturnDataTest {
    using {readSize} for uint256;
    function test_return_data() public {
        new Target();
        require(uint256(0).readSize() == 0, "aliased using function buffer");
    }
}"#,
        ),
        (
            "library",
            "library Reader { function size(uint256) internal view returns (uint256 n) { assembly { n := returndatasize() } } }",
            r#"
import {Reader} from '../src/Observe.sol';
contract ReturnDataTest {
    function test_return_data() public {
        new Target();
        require(Reader.size(0) == 0, "library buffer");
    }
}"#,
        ),
        (
            "parenthesized_library",
            "library Reader { function size() internal pure returns (uint256 n) { assembly { n := returndatasize() } } }",
            r#"
import {Reader} from '../src/Observe.sol';
contract ReturnDataTest {
    function test_return_data() public {
        new Target();
        require((Reader).size() == 0, "parenthesized library buffer");
    }
}"#,
        ),
        (
            "unary_operator",
            "type Word is uint256; using {size as -} for Word global; function size(Word) pure returns (Word) { uint256 n; assembly { n := returndatasize() } return Word.wrap(n); }",
            r#"
import {Word} from '../src/Observe.sol';
contract ReturnDataTest {
    function test_return_data() public {
        new Target();
        require(Word.unwrap(-Word.wrap(0)) == 0, "unary operator buffer");
    }
}"#,
        ),
        (
            "binary_operator",
            "type Word is uint256; using {size as +} for Word global; function size(Word, Word) pure returns (Word) { uint256 n; assembly { n := returndatasize() } return Word.wrap(n); }",
            r#"
import {Word} from '../src/Observe.sol';
contract ReturnDataTest {
    function test_return_data() public {
        new Target();
        require(Word.unwrap(Word.wrap(0) + Word.wrap(0)) == 0, "binary operator buffer");
    }
}"#,
        ),
        (
            "creation_code",
            "",
            r#"
contract ReturnDataTest {
    function ping() external pure returns (uint256) { return 77; }
    function test_return_data() public {
        this.ping();
        bytes memory code = type(Target).creationCode;
        uint256 value;
        assembly { returndatacopy(0, 0, 32) value := mload(0) }
        require(code.length > 0 && value == 77, "creation code buffer");
    }
}"#,
        ),
        (
            "modifier",
            "",
            r#"
contract ReturnDataTest {
    modifier check() {
        _;
        uint256 n;
        assembly { n := returndatasize() }
        require(n == 0, "modifier buffer");
    }
    function test_return_data() public check { new Target(); }
}"#,
        ),
        (
            "function_pointer",
            "function size() view returns (uint256 n) { assembly { n := returndatasize() } }",
            r#"
import {size} from '../src/Observe.sol';
contract ReturnDataTest {
    function test_return_data() public {
        function() internal view returns (uint256) observe = size;
        new Target();
        require(observe() == 0, "function pointer buffer");
    }
}"#,
        ),
    ] {
        prj.add_source("Observe.sol", helper);
        prj.add_test(
            "ReturnData.t.sol",
            &format!("import '../src/Target.sol'; {declarations}")
                .replace("test_return_data", &format!("test_{case}")),
        );
        for dynamic in [false, true] {
            prj.update_config(|config| config.dynamic_test_linking = dynamic);
            cmd.forge_fuse().args(["test", "--force"]).assert_success().stdout_eq(format!(
                r#"...
Ran 1 test for test/ReturnData.t.sol:ReturnDataTest
[PASS] test_{case}() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
"#
            ));
        }
    }
});

forgetest!(preprocess_inline_verbatim_diagnostics, |prj, cmd| {
    prj.add_source("Target.sol", "contract Target {}");
    for expression in ["let n := verbatim_0i_1o(hex\"3d\")", "verbatim_3i_0o(hex\"3e\", 0, 0, 0)"] {
        prj.add_test(
            "Verbatim.t.sol",
            &format!("import '../src/Target.sol'; contract VerbatimTest {{ function test_verbatim() public {{ new Target(); assembly {{ {expression} }} }} }}"),
        );
        prj.update_config(|config| config.dynamic_test_linking = false);
        let native = cmd
            .forge_fuse()
            .args(["build", "--force"])
            .assert_failure()
            .get_output()
            .stderr
            .clone();
        prj.update_config(|config| config.dynamic_test_linking = true);
        cmd.forge_fuse().args(["build", "--force"]).assert_failure().stderr_eq(native);
    }
});

forgetest!(preprocess_imported_constant_dependencies, |prj, cmd| {
    let target =
        "contract Target { function value() external pure returns (uint256) { return 11; } }";
    for (import, value) in [
        ("import {CODE} from '../src/Constants.sol';", "CODE"),
        ("import {CODE as ALIAS} from '../src/Constants.sol';", "ALIAS"),
        ("import {ALIAS} from '../src/Constants.sol';", "ALIAS"),
        ("import * as Constants from '../src/Constants.sol';", "Constants.CODE"),
    ] {
        prj.add_source(
            "Constants.sol",
            "import './Target.sol'; bytes constant CODE = type(Target).creationCode; bytes constant ALIAS = CODE;",
        );
        prj.add_test(
            "Constants.t.sol",
            &format!(
                r#"
{import}
import {{Target}} from '../src/Target.sol';
contract ConstantsTest {{
    function test_value() public {{
        bytes memory code = {value};
        address deployed;
        assembly {{ deployed := create(0, add(code, 32), mload(code)) }}
        require(Target(deployed).value() == 11, "value");
    }}
}}
"#
            ),
        );
        for dynamic in [false, true] {
            prj.update_config(|config| config.dynamic_test_linking = dynamic);
            prj.add_source("Target.sol", target);
            cmd.forge_fuse().args(["test", "--force"]).assert_success();
            cmd.forge_fuse().arg("test").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/Constants.t.sol:ConstantsTest
[PASS] test_value() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
            prj.add_source("Target.sol", &target.replace("11", "22"));
            for force in [false, true] {
                cmd.forge_fuse().arg("test");
                if force {
                    cmd.arg("--force");
                }
                cmd.assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Constants.t.sol:ConstantsTest
[FAIL: value] test_value() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
            }
            prj.add_source("Target.sol", target);
            cmd.forge_fuse().arg("test").assert_success();
        }
    }
});
