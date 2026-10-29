use alloy_primitives::U256;
use foundry_compilers::artifacts::output_selection::ContractOutputSelection;
use foundry_test_utils::util::OutputExt;
use itertools::Itertools;
use std::path::Path;

#[forgetest]
#[cfg(unix)]
fn debugger_selects_once_across_network_passes(prj: _) {
    use rexpect::{Encoding, process::wait::WaitStatus, reader::Options, spawn_with_options};

    const TIMEOUT_MS: u64 = 30_000;

    prj.add_test(
        "MultiNetwork.t.sol",
        r#"
contract MultiNetworkTest {
    function testDefault() public {}

    /// forge-config: default.networks.network = "tempo"
    function testTempo() public {}
}
"#,
    );

    let dump_path = prj.root().join("debug.json");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_forge"));
    command
        .current_dir(prj.root())
        .env_remove("CI")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["test", "--debug", "--dump"])
        .arg(&dump_path);

    let mut session = spawn_with_options(
        command,
        Options {
            timeout_ms: Some(TIMEOUT_MS),
            strip_ansi_escape_codes: true,
            encoding: Encoding::UTF8,
        },
    )
    .unwrap();

    session.exp_string("Select a test to debug:").unwrap();
    session.send("\x1b[B\x1b[B\r").unwrap();
    session.flush().unwrap();

    let output = session.exp_eof().unwrap();
    assert!(
        matches!(session.process.wait().unwrap(), WaitStatus::Exited(_, 0)),
        "debug command failed: {output}"
    );
    assert_eq!(
        output.matches("Select a test to debug:").count(),
        1,
        "selection picker repeated: {output}"
    );
    assert!(output.contains("[PASS] testTempo()"), "selected test did not run: {output}");
    assert!(!output.contains("[PASS] testDefault()"), "unselected test also ran: {output}");
    assert!(dump_path.exists(), "debugger dump was not created");
    serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(dump_path.join("trace.json")).unwrap(),
    )
    .unwrap();
}

// Sets up a debuggable test case.
// Run with `cargo test-debugger`.
#[forgetest]
#[ignore = "ran manually"]
#[expect(clippy::disallowed_macros, reason = "prints the setup for a manual debug session")]
fn manual_debug_setup(prj: _, cmd: _) {
    cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![""]).stderr_eq(
        str![[r#"
Warning: Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: https://github.com/foundry-rs/forge-std, tag: None)
...
    Installed forge-std[..]
    Initialized forge project

"#]],
    );

    prj.add_source("Counter2.sol", r#"
contract A {
    address public a;
    uint public b;
    int public c;
    bytes32 public d;
    bool public e;
    bytes public f;
    string public g;

    constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g) {
        a = _a;
        b = _b;
        c = _c;
        d = _d;
        e = _e;
        f = _f;
        g = _g;
    }

    function getA() public view returns (address) {
        return a;
    }

    function setA(address _a) public {
        a = _a;
    }
}"#,
    );

    let script = prj.add_script("Counter.s.sol", r#"
import "../src/Counter2.sol";
import "forge-std/Script.sol";
import "forge-std/Test.sol";

contract B is A {
    A public other;
    address public self = address(this);

    constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g)
        A(_a, _b, _c, _d, _e, _f, _g)
    {
        other = new A(_a, _b, _c, _d, _e, _f, _g);
    }
}

contract Script0 is Script, Test {
    function run() external {
        assertEq(uint256(1), uint256(1));

        vm.startBroadcast();
        B b = new B(msg.sender, 2 ** 32, -1 * (2 ** 32), keccak256(abi.encode(1)), true, "abcdef", "hello");
        assertEq(b.getA(), msg.sender);
        b.setA(tx.origin);
        assertEq(b.getA(), tx.origin);
        address _b = b.self();
        bytes32 _d = b.d();
        bytes32 _d2 = b.other().d();
    }
}"#,
    );

    cmd.forge_fuse().args(["build"]).assert_success();

    cmd.args([
        "script",
        script.to_str().unwrap(),
        "--root",
        prj.root().to_str().unwrap(),
        "--tc=Script0",
        "--debug",
    ]);
    eprintln!("root: {}", prj.root().display());
    let cmd_path = Path::new(cmd.cmd().get_program()).canonicalize().unwrap();
    let args = cmd.cmd().get_args().map(|s| s.to_str().unwrap()).format(" ");
    eprintln!(" cmd: {} {args}", cmd_path.display());
    std::mem::forget(prj);
}

// Drives a soldb session over stdin and checks a `vm.breakpoint` stop, a source stop, variables,
// frames, and the `--dump` bundle. Dynamic test linking rewrites `new Counter()` in the test, so
// the test file's lines come from the source the compiler saw.
#[forgetest]
fn debugger_soldb_session(prj: _, cmd: _) {
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        uint256 next = number + 1;
        number = next;
    }
}
"#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Counter} from "../src/Counter.sol";

interface Vm {
    function breakpoint(string calldata char) external;
}

contract CounterTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    Counter counter;

    function setUp() public {
        counter = new Counter();
    }

    function test_increment() public {
        counter.setNumber(41);
        vm.breakpoint("a");
        counter.increment();
        require(counter.number() == 42, "wrong number");
    }
}
"#,
    );

    cmd.args(["test", "--debug", "--mt", "test_increment"])
        .stdin("break src/Counter.sol:13\ncontinue\ncontinue\nvars\nbacktrace\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] test_increment() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
Loaded trace with 1125 steps.
Sources loaded for: CounterTest (0x7fa9385be102ac3eac297483dd6233d62b3e1496), Counter (0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f)
test/Counter.t.sol:10  (step 0/1124, pc 0, PUSH1, gas 1073664226)
   10 | contract CounterTest {
Breakpoint #2 set at src/Counter.sol:13
Breakpoint #1 hit at step 710, PC 529 if step >= 304 && step < 1125 && depth == 1
test/Counter.t.sol:20 in test_increment  (step 710/1124, pc 529, CALL, gas 1073668730)
   20 |         vm.breakpoint("a");
Breakpoint #2 hit at step 868, src/Counter.sol:13
src/Counter.sol:13 in increment  (step 868/1124, pc 167, DUP1, gas 1056889650)
   13 |         number = next;
warning: local variables are inferred from the legacy source map and the stack layout of solc's legacy code generator, not from compiler-reported variable locations
uint256 next = 42 [stack+2]
State:
uint256 number = 41 [slot 0x0]
#0  increment at src/Counter.sol:13  step 868, PC 167
#1  Counter at src/Counter.sol:4  step 799, PC 62
#2  test_increment at test/Counter.t.sol:21  step 765, PC 652
#3  CounterTest at test/Counter.t.sol:10  step 332, PC 51
Exiting debugger.

"#]]);

    // `--dump` writes a bundle that soldb's own tools read instead.
    let dump_dir = prj.root().join("soldb");
    cmd.forge_fuse()
        .args(["test", "--debug", "--mt", "test_increment", "--dump"])
        .arg(&dump_dir)
        .assert_success();
    let read_json = |path: &str| {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(dump_dir.join(path)).unwrap())
            .unwrap()
    };
    // Every program names the hash of its code, which depends on the compiled bytecode.
    let mut mapping = read_json("contracts.json");
    for entry in mapping["contracts"].as_array_mut().unwrap() {
        assert!(entry.as_object_mut().unwrap().remove("code_hash").unwrap().is_string());
    }
    assert_eq!(
        mapping,
        serde_json::json!({ "contracts": [
            {
                "address": "0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f",
                "name": "Counter",
                "debug_dir": "0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f",
            },
            {
                "address": "0x7fa9385be102ac3eac297483dd6233d62b3e1496",
                "name": "CounterTest",
                "debug_dir": "0x7fa9385be102ac3eac297483dd6233d62b3e1496",
            },
        ]})
    );
    assert!(dump_dir.join("0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f/combined.json").exists());
    assert!(dump_dir.join("src/Counter.sol").exists());
    let trace = read_json("trace.json");
    let calls = trace["artifacts"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| (call["call_type"].as_str().unwrap(), call["entry_step"].as_u64()))
        .collect::<Vec<_>>();
    assert_eq!(trace["backend"], "foundry");
    assert_eq!(trace["steps"].as_array().unwrap().len(), 1125);
    assert_eq!(
        calls,
        [
            ("CALL", Some(17)),
            // `vm.deployCode` runs no step of its own, so the creation it makes owns its steps.
            ("CALL", None),
            ("CALL", Some(304)),
            ("CALL", Some(469)),
            ("CALL", Some(711)),
            ("CALL", Some(766)),
            ("STATICCALL", Some(924))
        ]
    );
}

// Transient storage resets after setup, and the return buffer survives the following local steps.
#[forgetest]
fn debugger_soldb_transient_storage_and_return_data(prj: _, cmd: _) {
    prj.add_test(
        "TraceData.t.sol",
        r#"
contract TraceDataTest {
    function setUp() public {
        assembly { tstore(7, 99) }
    }

    function value() external pure returns (uint256) {
        return 42;
    }

    function test_data() public {
        uint256 result;
        assembly { result := tload(7) }
        require(result == 0);
        result = this.value();
        assembly { tstore(7, result) }
        assembly { result := tload(7) }
        require(result == 42);
    }
}
"#,
    );
    cmd.args(["test", "--debug", "--mt", "test_data"])
        .stdin("break test/TraceData.t.sol:16\nbreak test/TraceData.t.sol:20\ncontinue\ntransient\ncontinue\ntransient\nreturndata\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/TraceData.t.sol:TraceDataTest
[PASS] test_data() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
Loaded trace with 342 steps.
Sources loaded for: TraceDataTest (0x7fa9385be102ac3eac297483dd6233d62b3e1496)
test/TraceData.t.sol:4  (step 0/341, pc 0, PUSH1, gas 1073680956)
    4 | contract TraceDataTest {
Breakpoint #1 set at test/TraceData.t.sol:16
Breakpoint #2 set at test/TraceData.t.sol:20
Breakpoint #1 hit at step 96, test/TraceData.t.sol:16
test/TraceData.t.sol:16 in test_data  (step 96/341, pc 139, PUSH0, gas 1073720494)
   16 |         require(result == 0);
Transient storage: of 0x7fa9385be102ac3eac297483dd6233d62b3e1496
  0x7 = 0x0
Breakpoint #2 hit at step 332, test/TraceData.t.sol:20
test/TraceData.t.sol:20 in test_data  (step 332/341, pc 270, PUSH1, gas 1073719426)
   20 |         require(result == 42);
Transient storage: of 0x7fa9385be102ac3eac297483dd6233d62b3e1496
  0x7 = 0x2a
Return data: 32 bytes
0x000000000000000000000000000000000000000000000000000000000000002a
Exiting debugger.

"#]]);
}

// Opens a script in soldb and checks a stop inside a contract the script creates.
#[forgetest]
fn script_debugger_soldb_session(prj: _, cmd: _) {
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function increment() public {
        number = number + 1;
    }
}
"#,
    );
    prj.add_script(
        "Counter.s.sol",
        r#"
import {Counter} from "../src/Counter.sol";

contract CounterScript {
    function run() public {
        Counter counter = new Counter();
        counter.increment();
        counter.increment();
    }
}
"#,
    );

    cmd.args(["script", "script/Counter.s.sol", "--debug"])
        .stdin("break src/Counter.sol:8\ncontinue\ncontinue\nvars\nbacktrace\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Loaded trace with 396 steps.
Sources loaded for: CounterScript (0x9f7cf1d1f558e57ef88a59ac3d47214ef25b6a06), Counter (0x5aadfb43ef8daf45dd80f4676345b7676f1d70e3)
script/Counter.s.sol:6  (step 0/395, pc 0, PUSH1, gas 1073678100)
    6 | contract CounterScript {
Breakpoint #1 set at src/Counter.sol:8
Breakpoint #1 hit at step 167, src/Counter.sol:8
src/Counter.sol:8 in increment  (step 167/395, pc 92, PUSH1, gas 1056795911)
    8 |         number = number + 1;
Breakpoint #1 hit at step 314, src/Counter.sol:8
src/Counter.sol:8 in increment  (step 314/395, pc 92, PUSH1, gas 1056752761)
    8 |         number = number + 1;
warning: local variables are inferred from the legacy source map and the stack layout of solc's legacy code generator, not from compiler-reported variable locations
Variables: no variables in scope at PC 92
State:
uint256 number = 1 [slot 0x0]
#0  increment at src/Counter.sol:8  step 314, PC 92
#1  Counter at src/Counter.sol:4  step 308, PC 47
#2  run at script/Counter.s.sol:10  step 279, PC 259
#3  CounterScript at script/Counter.s.sol:6  step 40, PC 40
Exiting debugger.

"#]]);
}

// Code replaced with `vm.etch` maps through its own program, not the one that ran at the address
// first.
#[forgetest]
fn debugger_soldb_etched_code(prj: _, cmd: _) {
    prj.add_test(
        "Etch.t.sol",
        r#"
interface Vm {
    function etch(address target, bytes calldata newRuntimeBytecode) external;
}

contract Trusted {
    function trustedOnly() external pure returns (uint256) {
        return trustedInternal();
    }

    function trustedInternal() internal pure returns (uint256) {
        return 1;
    }
}

contract Payload {
    function attackerEntry() external pure returns (uint256) {
        return payloadInternal();
    }

    function payloadInternal() internal pure returns (uint256) {
        return 42;
    }
}

contract EtchTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_etch() public {
        Trusted trusted = new Trusted();
        require(trusted.trustedOnly() == 1);
        vm.etch(address(trusted), type(Payload).runtimeCode);
        require(Payload(address(trusted)).attackerEntry() == 42);
    }
}
"#,
    );

    cmd.args(["test", "--debug", "--mt", "test_etch"])
        .stdin("break test/Etch.t.sol:14\nbreak test/Etch.t.sol:24\ncontinue\nbacktrace\ncontinue\nbacktrace\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Etch.t.sol:EtchTest
[PASS] test_etch() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
Loaded trace with 817 steps.
Sources loaded for: EtchTest (0x7fa9385be102ac3eac297483dd6233d62b3e1496), Trusted (0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f), Payload (0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f)
test/Etch.t.sol:28  (step 0/816, pc 0, PUSH1, gas 1073667580)
   28 | contract EtchTest {
Breakpoint #1 set at test/Etch.t.sol:14
Breakpoint #2 set at test/Etch.t.sol:24
Breakpoint #1 hit at step 159, test/Etch.t.sol:14
test/Etch.t.sol:14 in trustedInternal  (step 159/816, pc 82, PUSH1, gas 1056819391)
   14 |         return 1;
#0  trustedInternal at test/Etch.t.sol:14  step 159, PC 82
#1  trustedOnly at test/Etch.t.sol:10  step 156, PC 74
#2  Trusted at test/Etch.t.sol:8  step 147, PC 37
#3  test_etch at test/Etch.t.sol:33  step 123, PC 155
#4  EtchTest at test/Etch.t.sol:28  step 40, PC 39
Breakpoint #2 hit at step 643, test/Etch.t.sol:24
test/Etch.t.sol:24 in payloadInternal  (step 643/816, pc 82, PUSH1, gas 1056815059)
   24 |         return 42;
#0  payloadInternal at test/Etch.t.sol:24  step 643, PC 82
#1  attackerEntry at test/Etch.t.sol:20  step 640, PC 74
#2  Payload at test/Etch.t.sol:18  step 631, PC 37
#3  test_etch at test/Etch.t.sol:35  step 607, PC 448
#4  EtchTest at test/Etch.t.sol:28  step 40, PC 39
Exiting debugger.

"#]]);

    // The bundle keeps both programs run at the address, each with its own directory.
    let dump_dir = prj.root().join("soldb");
    cmd.forge_fuse()
        .args(["test", "--debug", "--mt", "test_etch", "--dump"])
        .arg(&dump_dir)
        .assert_success();
    let mapping = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(dump_dir.join("contracts.json")).unwrap(),
    )
    .unwrap();
    let mut names = bundle_programs(&mapping, "0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f");
    names.sort_unstable();
    assert_eq!(names, ["Payload", "Trusted"]);
}

/// The names of the programs a bundle's `contracts.json` maps at `address`, after checking that
/// each has its own directory and code hash.
fn bundle_programs(mapping: &serde_json::Value, address: &str) -> Vec<String> {
    let entries = mapping["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["address"] == address)
        .collect::<Vec<_>>();
    let dirs = entries
        .iter()
        .map(|entry| entry["debug_dir"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    let hashes =
        entries.iter().map(|entry| entry["code_hash"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(dirs.len(), entries.len());
    assert_eq!(hashes.iter().collect::<std::collections::BTreeSet<_>>().len(), entries.len());
    entries.iter().map(|entry| entry["name"].as_str().unwrap().to_string()).collect()
}

// Runtimes of one contract that differ only in immutables keep separate bundle entries when one
// replaces the other at an address.
#[forgetest]
fn debugger_soldb_dump_keeps_etched_runtimes(prj: _, cmd: _) {
    prj.add_test(
        "Variant.t.sol",
        r#"
interface Vm {
    function etch(address target, bytes calldata newRuntimeBytecode) external;
}

contract Variant {
    uint256 private immutable value;

    constructor(uint256 value_) {
        value = value_;
    }

    function get() external view returns (uint256) {
        return value;
    }
}

contract VariantTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_etch_variant() public {
        Variant first = new Variant(1);
        Variant second = new Variant(2);
        require(first.get() == 1);
        vm.etch(address(first), address(second).code);
        require(first.get() == 2);
    }
}
"#,
    );

    let dump_dir = prj.root().join("soldb");
    cmd.args(["test", "--debug", "--mt", "test_etch_variant", "--dump"])
        .arg(&dump_dir)
        .assert_success();
    let mapping = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(dump_dir.join("contracts.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        bundle_programs(&mapping, "0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f"),
        ["Variant", "Variant"]
    );
}

// With dynamic test linking, `new Child()` runs as a `vm.deployCode` call around the creation, and
// the constructor's steps and storage writes still belong to the creation.
#[forgetest]
fn debugger_soldb_constructor_under_deploy_code(prj: _, cmd: _) {
    prj.add_source(
        "Child.sol",
        r#"
contract Child {
    uint256 public number;

    constructor() {
        number = 41;
    }

    function bump() external {
        number += 1;
    }
}
"#,
    );
    prj.add_test(
        "Child.t.sol",
        r#"
import {Child} from "../src/Child.sol";

contract ChildTest {
    function test_child() public {
        Child child = new Child();
        child.bump();
    }
}
"#,
    );

    cmd.args(["test", "--debug", "--mt", "test_child"])
        .stdin("break src/Child.sol:8\nbreak src/Child.sol:12\ncontinue\nbacktrace\ncontinue\nprint number\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Child.t.sol:ChildTest
[PASS] test_child() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
Loaded trace with 441 steps.
Sources loaded for: ChildTest (0x7fa9385be102ac3eac297483dd6233d62b3e1496), Child (0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f)
test/Child.t.sol:6  (step 0/440, pc 0, PUSH1, gas 1073679004)
    6 | contract ChildTest {
Breakpoint #1 set at src/Child.sol:8
Breakpoint #2 set at src/Child.sol:12
Breakpoint #1 hit at step 160, src/Child.sol:8
src/Child.sol:8 in constructor  (step 160/440, pc 16, PUSH1, gas 1040394615)
    8 |         number = 41;
#0  constructor at src/Child.sol:8  step 160, PC 16
#1  Child at src/Child.sol:4  step 152, PC 4
#2  Child at src/Child.sol:4  step 150, PC 0
#3  test_child at test/Child.t.sol:8  step 149, PC 144
#4  ChildTest at test/Child.t.sol:6  step 40, PC 40
Breakpoint #2 hit at step 354, src/Child.sol:12
src/Child.sol:12 in bump  (step 354/440, pc 87, PUSH1, gas 1056769917)
   12 |         number += 1;
warning: local variables are inferred from the legacy source map and the stack layout of solc's legacy code generator, not from compiler-reported variable locations
uint256 number = 41 [slot 0x0]
Exiting debugger.

"#]]);
}

// A plain build writes build infos without the compiler input and the cache does not track them,
// so a debug run recompiles to read the test as dynamic test linking rewrote it. The line after
// the rewritten `new Child()` would otherwise map to the wrong text.
#[forgetest]
fn debugger_recompiles_cached_build_without_sources(prj: _, cmd: _) {
    prj.update_config(|config| config.extra_output = vec![ContractOutputSelection::StorageLayout]);
    prj.add_source(
        "Child.sol",
        r#"
contract Child {
    uint256 public number;

    function bump() external {
        number += 1;
    }
}
"#,
    );
    prj.add_test(
        "Child.t.sol",
        r#"
import {Child} from "../src/Child.sol";

contract ChildTest {
    function test_child() public {
        Child child = new Child();
        child.bump();
    }
}
"#,
    );
    cmd.arg("build").assert_success();
    // The rebuild keeps the persisted fuzz failures.
    let persisted = prj.root().join("cache/fuzz/failures");
    std::fs::create_dir_all(persisted.parent().unwrap()).unwrap();
    std::fs::write(&persisted, "").unwrap();

    cmd.forge_fuse()
        .args(["test", "--debug", "--mt", "test_child"])
        .stdin("break test/Child.t.sol:9\ncontinue\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Child.t.sol:ChildTest
[PASS] test_child() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
Loaded trace with 435 steps.
Sources loaded for: ChildTest (0x7fa9385be102ac3eac297483dd6233d62b3e1496), Child (0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f)
test/Child.t.sol:6  (step 0/434, pc 0, PUSH1, gas 1073679004)
    6 | contract ChildTest {
Breakpoint #1 set at test/Child.t.sol:9
Breakpoint #1 hit at step 284, test/Child.t.sol:9
test/Child.t.sol:9 in test_child  (step 284/434, pc 198, DUP1, gas 1073566681)
    9 |         child.bump();
Exiting debugger.

"#]]);
    assert!(persisted.exists());
}

// The fuzz campaign runs without the debug recorder, and the counterexample runs again with it.
#[forgetest]
fn debugger_records_fuzz_counterexample(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.runs = 1024;
        config.fuzz.seed = Some(U256::from(1));
    });
    prj.add_test(
        "Fuzz.t.sol",
        r#"
contract FuzzTest {
    function testFuzz_small(uint256 x) public pure {
        require(x < 1000, "too big");
    }
}
"#,
    );

    let dump_dir = prj.root().join("soldb");
    let output = cmd
        .args(["test", "--debug", "--mt", "testFuzz_small", "--dump"])
        .arg(&dump_dir)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let calldata = output.split("calldata=").nth(1).unwrap().split(' ').next().unwrap();
    let trace = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(dump_dir.join("trace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(trace["input_data"], calldata);
    assert_eq!(trace["success"], false);
}
