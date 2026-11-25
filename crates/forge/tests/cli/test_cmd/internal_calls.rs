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

// A type that a function names through an import alias is not the declaration of that name
// elsewhere, so its type is unknown rather than taken from the other declaration.
#[forgetest]
fn import_alias_types_are_unknown(prj: _, cmd: _) {
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
    │   ├─ [..] Alias::inner(<unknown>)
    │   │   └─ ← <unknown>
    │   └─ ← [Return] 10
    └─ ← [Stop]
...
"#]]);
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
    cmd.args(["test", "-vvvv", "--decode-internal", "--match-path", "test/A.t.sol"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Traces:
  [..] ATest::test_a()
...
    ├─ [..] Base::helper(uint256)(5)
    │   └─ ← 6
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "-vvvv", "--decode-internal", "--match-path", "test/B.t.sol"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Traces:
  [..] BTest::test_b()
...
    ├─ [..] Base::helper(uint256)(6)
    │   └─ ← 7
...
"#]]);
}
