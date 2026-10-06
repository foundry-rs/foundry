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
    serde_json::from_slice::<serde_json::Value>(&std::fs::read(dump_path).unwrap()).unwrap();
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
#[cfg(feature = "soldb")]
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

    cmd.args(["test", "--debug", "--debugger", "soldb", "--mt", "test_increment"])
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
Loaded trace with 821 steps.
Sources loaded for: CounterTest (0x7fa9385be102ac3eac297483dd6233d62b3e1496), Counter (0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f)
test/Counter.t.sol:10  (step 0/820, pc 0, PUSH1, gas 1073720760)
   10 | contract CounterTest {
Breakpoint #2 set at src/Counter.sol:13
Breakpoint #1 hit at step 406, PC 529 if step == 406
test/Counter.t.sol:20 in test_increment  (step 406/820, pc 529, CALL, gas 1073668730)
   20 |         vm.breakpoint("a");
Breakpoint #2 hit at step 564, src/Counter.sol:13
src/Counter.sol:13 in increment  (step 564/820, pc 167, DUP1, gas 1056889650)
   13 |         number = next;
warning: local variables are inferred from the legacy source map and the stack layout of solc's legacy code generator, not from compiler-reported variable locations
uint256 next = 42 [stack+2]
State:
uint256 number = 41 [slot 0x0]
#0  increment at src/Counter.sol:13  step 564, PC 167
#1  Counter at src/Counter.sol:4  step 495, PC 62
#2  test_increment at test/Counter.t.sol:21  step 461, PC 652
#3  CounterTest at test/Counter.t.sol:10  step 28, PC 51
Exiting debugger.

"#]]);

    // `--dump` writes a bundle that soldb's own tools read instead.
    let dump_dir = prj.root().join("soldb");
    cmd.forge_fuse()
        .args(["test", "--debug", "--debugger", "soldb", "--mt", "test_increment", "--dump"])
        .arg(&dump_dir)
        .assert_success();
    let read_json = |path: &str| {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(dump_dir.join(path)).unwrap())
            .unwrap()
    };
    assert_eq!(
        read_json("contracts.json"),
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
        .map(|call| (call["call_type"].as_str().unwrap(), call["entry_step"].as_u64().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(trace["backend"], "foundry");
    assert_eq!(trace["steps"].as_array().unwrap().len(), 821);
    assert_eq!(
        calls,
        [("CALL", 0), ("CALL", 165), ("CALL", 407), ("CALL", 462), ("STATICCALL", 620)]
    );
}

// Opens a script in soldb and checks a stop inside a contract the script creates.
#[forgetest]
#[cfg(feature = "soldb")]
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

    cmd.args(["script", "script/Counter.s.sol", "--debug", "--debugger", "soldb"])
        .stdin("break src/Counter.sol:8\ncontinue\ncontinue\nvars\nbacktrace\nquit\n")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Loaded trace with 379 steps.
Sources loaded for: CounterScript (0x9f7cf1d1f558e57ef88a59ac3d47214ef25b6a06), Counter (0x5aadfb43ef8daf45dd80f4676345b7676f1d70e3)
script/Counter.s.sol:6  (step 0/378, pc 0, PUSH1, gas 1073720760)
    6 | contract CounterScript {
Breakpoint #1 set at src/Counter.sol:8
Breakpoint #1 hit at step 150, src/Counter.sol:8
src/Counter.sol:8 in increment  (step 150/378, pc 92, PUSH1, gas 1056795911)
    8 |         number = number + 1;
Breakpoint #1 hit at step 297, src/Counter.sol:8
src/Counter.sol:8 in increment  (step 297/378, pc 92, PUSH1, gas 1056752761)
    8 |         number = number + 1;
warning: local variables are inferred from the legacy source map and the stack layout of solc's legacy code generator, not from compiler-reported variable locations
Variables: no variables in scope at PC 92
State:
uint256 number = 1 [slot 0x0]
#0  increment at src/Counter.sol:8  step 297, PC 92
#1  Counter at src/Counter.sol:4  step 291, PC 47
#2  run at script/Counter.s.sol:10  step 262, PC 259
#3  CounterScript at script/Counter.s.sol:6  step 23, PC 40
Exiting debugger.

"#]]);
}

// Requests the compiler's ETHDebug programs for an unoptimized via-IR build, and writes them into
// the soldb bundle, where soldb prefers them over source maps.
#[forgetest]
#[cfg(feature = "soldb")]
fn debugger_soldb_ethdebug(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.via_ir = true;
        config.optimizer = Some(false);
    });
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
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Counter} from "../src/Counter.sol";

contract CounterTest {
    function test_increment() public {
        Counter counter = new Counter();
        counter.increment();
    }
}
"#,
    );

    let dump_dir = prj.root().join("soldb");
    cmd.args(["test", "--debug", "--debugger", "soldb", "--mt", "test_increment", "--dump"])
        .arg(&dump_dir)
        .assert_success();
    let program =
        dump_dir.join("0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f/Counter_ethdebug-runtime.json");
    let program =
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(program).unwrap()).unwrap();
    assert_eq!(program["environment"], "call");
    assert_eq!(program["contract"]["name"], "Counter");
    assert!(!program["instructions"].as_array().unwrap().is_empty());
}
