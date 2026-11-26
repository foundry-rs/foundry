//! Tests for internal call detection with `--decode-internal`.

use foundry_test_utils::{TestCommand, TestProject, file, snapbox::IntoData, str};

/// Compiles the fixture in the given mode and compares the `-vvvv --decode-internal` output.
#[track_caller]
fn assert_internal_calls(
    prj: &TestProject,
    cmd: &mut TestCommand,
    (name, source): (&str, &str),
    (via_ir, optimize): (bool, bool),
    expected: impl IntoData,
) {
    prj.update_config(|config| {
        config.via_ir = via_ir;
        config.optimizer = Some(optimize);
        config.optimizer_runs = Some(200);
    });
    prj.add_test(name, source);
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(expected);
}

const TARGET: (&str, &str) =
    ("Target.t.sol", include_str!("../../fixtures/internal_calls/Target.t.sol"));
const PARAMS: (&str, &str) =
    ("Params.t.sol", include_str!("../../fixtures/internal_calls/Params.t.sol"));
const PUBLIC: (&str, &str) =
    ("Public.t.sol", include_str!("../../fixtures/internal_calls/Public.t.sol"));
const MATRIX: (&str, &str) =
    ("Matrix.t.sol", include_str!("../../fixtures/internal_calls/Matrix.t.sol"));
const LIBRARY: (&str, &str) =
    ("Library.t.sol", include_str!("../../fixtures/internal_calls/Library.t.sol"));
const C: (&str, &str) = ("C.t.sol", include_str!("../../fixtures/internal_calls/C.t.sol"));

#[forgetest]
fn target_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (false, false),
        file!["../../fixtures/internal_calls/target_legacy.txt"],
    );
}

#[forgetest]
fn target_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (false, true),
        file!["../../fixtures/internal_calls/target_legacy_optimized.txt"],
    );
}

#[forgetest]
fn target_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (true, false),
        file!["../../fixtures/internal_calls/target_via_ir.txt"],
    );
}

#[forgetest]
fn target_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        TARGET,
        (true, true),
        file!["../../fixtures/internal_calls/target_via_ir_optimized.txt"],
    );
}

#[forgetest]
fn c_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (false, false),
        file!["../../fixtures/internal_calls/c_legacy.txt"],
    );
}

#[forgetest]
fn c_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (false, true),
        file!["../../fixtures/internal_calls/c_legacy_optimized.txt"],
    );
}

#[forgetest]
fn c_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (true, false),
        file!["../../fixtures/internal_calls/c_via_ir.txt"],
    );
}

#[forgetest]
fn c_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        C,
        (true, true),
        file!["../../fixtures/internal_calls/c_via_ir_optimized.txt"],
    );
}

// Calls that need more than the function table: a function the call ends in, a public function
// that a fallback function calls, and struct parameters.
#[forgetest]
fn edge_cases(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        ("Edge.t.sol", include_str!("../../fixtures/internal_calls/Edge.t.sol")),
        (false, false),
        file!["../../fixtures/internal_calls/edge.txt"],
    );
}

// Parameters that take two stack words: calldata arrays of any element, `bytes` and `string`, and
// external function types. The parameters after them keep their values.
#[forgetest]
fn params_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        PARAMS,
        (false, false),
        file!["../../fixtures/internal_calls/params_legacy.txt"],
    );
}

#[forgetest]
fn params_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        PARAMS,
        (true, false),
        file!["../../fixtures/internal_calls/params_via_ir.txt"],
    );
}

// A public function that calls itself, or another public function of the same name, shows its
// inner calls, also if the compiler inlined the dispatcher.
#[forgetest]
fn public_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        PUBLIC,
        (false, false),
        file!["../../fixtures/internal_calls/public_legacy.txt"],
    );
}

#[forgetest]
fn public_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        PUBLIC,
        (false, true),
        file!["../../fixtures/internal_calls/public_legacy_optimized.txt"],
    );
}

#[forgetest]
fn public_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        PUBLIC,
        (true, false),
        file!["../../fixtures/internal_calls/public_via_ir.txt"],
    );
}

#[forgetest]
fn public_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        PUBLIC,
        (true, true),
        file!["../../fixtures/internal_calls/public_via_ir_optimized.txt"],
    );
}

// Every combination of a public function that calls itself or an overload of itself, which the
// dispatcher's entry into the external function must not hide.
#[forgetest]
fn matrix_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        MATRIX,
        (false, false),
        file!["../../fixtures/internal_calls/matrix_legacy.txt"],
    );
}

#[forgetest]
fn matrix_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        MATRIX,
        (false, true),
        file!["../../fixtures/internal_calls/matrix_legacy_optimized.txt"],
    );
}

#[forgetest]
fn matrix_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        MATRIX,
        (true, false),
        file!["../../fixtures/internal_calls/matrix_via_ir.txt"],
    );
}

#[forgetest]
fn matrix_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        MATRIX,
        (true, true),
        file!["../../fixtures/internal_calls/matrix_via_ir_optimized.txt"],
    );
}

// The functions that a library runs for a delegatecall are entered by its dispatcher: library ABIs
// spell enums, structs and contracts by qualified name, and leave out the functions with `storage`
// parameters, so the selector of the call does not name them in the ABI.
#[forgetest]
fn library_legacy(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        LIBRARY,
        (false, false),
        file!["../../fixtures/internal_calls/library_legacy.txt"],
    );
}

#[forgetest]
fn library_legacy_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        LIBRARY,
        (false, true),
        file!["../../fixtures/internal_calls/library_legacy_optimized.txt"],
    );
}

#[forgetest]
fn library_via_ir(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        LIBRARY,
        (true, false),
        file!["../../fixtures/internal_calls/library_via_ir.txt"],
    );
}

#[forgetest]
fn library_via_ir_optimized(prj: _, cmd: _) {
    assert_internal_calls(
        &prj,
        &mut cmd,
        LIBRARY,
        (true, true),
        file!["../../fixtures/internal_calls/library_via_ir_optimized.txt"],
    );
}

// A type that a function names through an import alias is the aliased type, and not the
// declaration of that name elsewhere in the project.
#[forgetest]
fn import_alias_types_are_the_aliased_type(prj: _, cmd: _) {
    prj.add_source("Types.sol", "type TokenId is uint256;");
    prj.add_source("Other.sol", "type Amount is address;");
    prj.add_source(
        "Alias.sol",
        r#"
import {TokenId as Amount} from "./Types.sol";

contract Alias {
    function inner(Amount a) internal pure returns (Amount) {
        return a;
    }

    function run(uint256 v) external pure returns (uint256) {
        return Amount.unwrap(inner(Amount.wrap(v)));
    }
}
"#,
    );
    prj.add_test(
        "Alias.t.sol",
        r#"
import {Alias} from "src/Alias.sol";

contract AliasTest {
    function test_alias() public {
        new Alias().run(10);
    }
}
"#,
    );
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(str![[r#"
...
Traces:
  [..] AliasTest::test_alias()
...
    ├─ [..] Alias::run(10) [staticcall]
    │   ├─ [..] Alias::inner(uint256)(10)
    │   │   └─ ← 10
    │   └─ ← [Return] 10
    └─ ← [Stop]
...
"#]]);
}

// A qualified type name such as `Types.Amount` is the type that the name resolves to, and not a
// type that the contract or the file of the function declares under the last segment of the name.
#[forgetest]
fn qualified_type_of_one_declaration(prj: _, cmd: _) {
    qualified_types(&prj, "");
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(str![[r#"
...
    ├─ [..] Qualified::run(1606938044258990275541962092341162602522202993782792835301386 [..]) [staticcall]
    │   ├─ [..] Qualified::inner(uint256)(1606938044258990275541962092341162602522202993782792835301386 [..])
    │   │   └─ ← 1606938044258990275541962092341162602522202993782792835301387 [..]
    │   └─ ← [Return] 1606938044258990275541962092341162602522202993782792835301387 [..]
...
"#]]);
}

#[forgetest]
fn qualified_type_with_a_local_declaration_of_the_name(prj: _, cmd: _) {
    qualified_types(&prj, "type Amount is address;");
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(str![[r#"
...
    ├─ [..] Qualified::run(1606938044258990275541962092341162602522202993782792835301386 [..]) [staticcall]
    │   ├─ [..] Qualified::inner(uint256)(1606938044258990275541962092341162602522202993782792835301386 [..])
    │   │   └─ ← 1606938044258990275541962092341162602522202993782792835301387 [..]
    │   └─ ← [Return] 1606938044258990275541962092341162602522202993782792835301387 [..]
...
"#]]);
}

/// A project where `Qualified` names `Types.Amount`, a `uint256` wrapper, and declares `local`.
fn qualified_types(prj: &TestProject, local: &str) {
    prj.add_source("Types.sol", "type Amount is uint256;");
    prj.add_source(
        "Qualified.sol",
        &format!(
            r#"
import "./Types.sol" as Types;

contract Qualified {{
    {local}

    function inner(Types.Amount a) internal pure returns (Types.Amount) {{
        return Types.Amount.wrap(Types.Amount.unwrap(a) + 1);
    }}

    function run(uint256 v) external pure returns (uint256) {{
        return Types.Amount.unwrap(inner(Types.Amount.wrap(v)));
    }}
}}
"#
        ),
    );
    prj.add_test(
        "Qualified.t.sol",
        r#"
import {Qualified} from "src/Qualified.sol";

contract QualifiedTest {
    function test_qualified() public {
        new Qualified().run((1 << 200) + 10);
    }
}
"#,
    );
}

// solc before 0.8.5 has no `functionDebugData`, so every function, Solidity or Yul, comes from the
// jump markers of the source map.
#[forgetest]
fn old_solc_without_function_debug_data(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Version("0.8.4".parse().unwrap()));
    });
    prj.add_test(
        "Old.t.sol",
        r#"pragma solidity 0.8.4;

contract Old {
    function inner(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function asm(uint256 x) internal pure returns (uint256 r) {
        assembly {
            function twice(a) -> b { b := add(a, a) }
            r := twice(x)
        }
    }

    function run(uint256 x) external pure returns (uint256) {
        return inner(x, 2) + asm(x);
    }
}

contract OldTest {
    function test_old() public {
        new Old().run(3);
    }
}
"#,
    );
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(str![[r#"
...
Traces:
  [..] OldTest::test_old()
    ├─ [..] → new Old@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 516 bytes of code
    ├─ [..] Old::run(3) [staticcall]
    │   ├─ [..] Old::asm(uint256)(3)
    │   │   ├─ [..] Old::twice(3)
    │   │   │   └─ ← 6
    │   │   └─ ← 6
    │   ├─ [..] Old::inner(uint256,uint256)(3, 2)
    │   │   └─ ← 5
    │   └─ ← [Return] 11
    └─ ← [Stop]
...
"#]]);
}

// A test file that two solc versions compile is edited for each of them by dynamic test linking,
// and the edits differ with the version. Each build maps its source locations with its own edits.
#[forgetest]
fn builds_edit_a_shared_file_differently(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.solc = None;
        config.auto_detect_solc = true;
        config.via_ir = true;
        config.dynamic_test_linking = true;
    });
    prj.add_raw_source(
        "Counter.sol",
        "pragma solidity >=0.8.20;\ncontract Counter { uint256 public n; }\n",
    );
    prj.add_raw_test(
        "Base.t.sol",
        r#"pragma solidity >=0.8.20;
import {Counter} from "src/Counter.sol";

abstract contract Base {
    function make() internal returns (Counter) {
        return new Counter();
    }

    function helper(uint256 x) internal pure returns (uint256) {
        return x + 1;
    }
}
"#,
    );
    prj.add_raw_test(
        "A.t.sol",
        r#"pragma solidity =0.8.20;
import {Base} from "./Base.t.sol";

contract ATest is Base {
    function test_a() public {
        make();
        helper(5);
    }
}
"#,
    );
    prj.add_raw_test(
        "B.t.sol",
        r#"pragma solidity =0.8.30;
import {Base} from "./Base.t.sol";

contract BTest is Base {
    function test_b() public {
        make();
        helper(6);
    }
}
"#,
    );
    // One run compiles both builds, and which of them is first is not fixed. With one thread the
    // two tests print in order.
    cmd.args(["test", "-vvvv", "--decode-internal", "--threads", "1"]).assert_success().stdout_eq(
        str![[r#"
...
Traces:
  [..] ATest::test_a()
...
    ├─ [..] Base::helper(uint256)(5)
    │   └─ ← 6
...
Traces:
  [..] BTest::test_b()
...
    ├─ [..] Base::helper(uint256)(6)
    │   └─ ← 7
...
"#]],
    );
}
