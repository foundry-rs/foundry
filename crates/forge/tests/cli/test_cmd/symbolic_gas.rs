use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness};
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, snapbox::assert_data_eq, str};

// `vm.startSnapshotGas`/`vm.stopSnapshotGas` return an upper bound on the gas of every path. The
// bound for `work` is reached: one gas less has a counterexample that replays concretely.
#[forgetest_init]
fn symbolic_gas_snapshot_bounds_isolated_call(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_gas_snapshot_bounds_isolated_call");

    prj.add_test(
        "SymbolicGas.t.sol",
        r#"
import "forge-std/Test.sol";

contract Vault {
    uint256 public total;

    function work(uint256 n) external {
        if (n > 5) total = n;
    }
}

contract SymbolicGas is Test {
    Vault vault;

    function setUp() public {
        vault = new Vault();
    }

    function checkWorkGas(uint256 n) public {
        vm.startSnapshotGas("work");
        vault.work(n);
        assertLe(vm.stopSnapshotGas(), 49218);
    }

    function checkWorkGasBelowMax(uint256 n) public {
        vm.startSnapshotGas("work");
        vault.work(n);
        assertLe(vm.stopSnapshotGas(), 49217);
    }

    function checkCreateGasIncomplete() public {
        vm.startSnapshotGas("create");
        new Vault();
        vm.stopSnapshotGas();
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicGas"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicGas.t.sol:SymbolicGas
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic gas for CREATE not modeled] checkCreateGasIncomplete() ([METRICS])
[PASS] checkWorkGas(uint256) ([METRICS])
[FAIL: assertion failed: 49218 > 49217; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkWorkGasBelowMax(uint256) ([METRICS])
...
"#]]);
}

// Without a limit, symbolic tests write the largest value of each gas snapshot to the snapshot
// file. Each value is replayed concretely with the worst-case input. The storage cases price
// fresh and existing slots and the refund for clearing a slot.
#[forgetest_init]
fn symbolic_gas_snapshot_reports_worst_case(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_gas_snapshot_reports_worst_case");

    prj.add_test(
        "SymbolicGasSearch.t.sol",
        r#"
import "forge-std/Test.sol";

contract Vault {
    uint256 public total;

    function work(uint256 n) external {
        if (n > 5) total = n;
    }
}

contract Ledger {
    uint256 public total = 1;
    mapping(address => uint256) public balances;

    function record(address account, uint256 amount) external {
        balances[account] = amount;
        total = amount;
    }
}

contract SymbolicGasSearch is Test {
    Vault vault;
    Ledger ledger;

    function setUp() public {
        vault = new Vault();
        ledger = new Ledger();
    }

    function checkWorkGas(uint256 n) public {
        vm.startSnapshotGas("work");
        vault.work(n);
        vm.stopSnapshotGas();
    }

    function checkRecordGas() public {
        vm.startSnapshotGas("recordNew");
        ledger.record(address(1), 5);
        vm.stopSnapshotGas();
        vm.startSnapshotGas("recordClear");
        ledger.record(address(1), 0);
        vm.stopSnapshotGas();
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicGasSearch"]))
        .success()
        .stdout_eq(str![[r#"
...
[PASS] checkRecordGas() ([METRICS])
[PASS] checkWorkGas(uint256) ([METRICS])
...
"#]]);

    let snapshot = std::fs::read_to_string(prj.root().join("snapshots/SymbolicGasSearch.json"));
    assert_data_eq!(
        snapshot.unwrap(),
        str![[r#"
{
  "recordClear": "26491",
  "recordNew": "54521",
  "work": "49215"
}
"#]]
    );
}

// `@custom:max-gas` on a function of a contract deployed in `setUp` is checked for one
// transaction with symbolic arguments. A limit below the worst case fails with the worst-case
// input, which replays concretely.
#[forgetest_init]
fn symbolic_gas_limit_tags(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_gas_limit_tags");

    prj.add_test(
        "SymbolicGasLimits.t.sol",
        r#"
import "forge-std/Test.sol";

contract Bank {
    mapping(address => uint256) public balances;
    address[] public depositors;

    /// @custom:max-gas 100_000
    function deposit(uint256 amount) external {
        if (balances[msg.sender] == 0) depositors.push(msg.sender);
        balances[msg.sender] += amount;
    }

    /// @custom:max-gas 30000
    function payout() external {
        for (uint256 i; i < depositors.length; i++) balances[depositors[i]] = 0;
    }
}

contract SymbolicGasLimits is Test {
    Bank bank;

    function setUp() public {
        bank = new Bank();
        for (uint160 i = 1; i <= 3; i++) {
            vm.prank(address(i));
            bank.deposit(10);
        }
    }

    function testDeposit() public {
        bank.deposit(1);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicGasLimits"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicGasLimits.t.sol:SymbolicGasLimits
[PASS] maxGas:Bank.deposit(uint256) ([METRICS])
[FAIL: gas 36862 exceeds @custom:max-gas 30000; worst-case input replays at 36862; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] maxGas:Bank.payout() ([METRICS])
[PASS] testDeposit() ([GAS])
...
"#]]);
}

// `forge build --gas-limits` checks `@custom:max-gas` without tests: from any storage state, with
// any sender and arguments, and with unknown immutables. Each worst case replays concretely with
// the storage and immutables it assumes. A call to other code has no bound.
#[forgetest_init]
fn build_checks_gas_limits(prj: _, cmd: _) {
    skip_unless_z3!("build_checks_gas_limits");

    prj.add_source(
        "Vault.sol",
        r#"
interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

contract Vault {
    IERC20 public immutable token;
    address public owner;
    uint256 public fee;
    mapping(address => uint256) public balances;

    constructor(IERC20 token_) {
        token = token_;
        owner = msg.sender;
    }

    /// @custom:max-gas 50_000
    function setFee(uint256 fee_) external {
        require(msg.sender == owner, "owner");
        fee = fee_;
    }

    /// @custom:max-gas 60_000
    function credit(address account, uint256 amount) external {
        require(msg.sender == owner, "owner");
        balances[account] += amount;
    }

    /// @custom:max-gas 40_000
    function creditTight(address account, uint256 amount) external {
        require(msg.sender == owner, "owner");
        balances[account] += amount;
    }

    /// @custom:max-gas 100_000
    function withdraw(uint256 amount) external {
        balances[msg.sender] -= amount;
        token.transfer(msg.sender, amount);
    }
}
"#,
    );

    cmd.args(["build", "--gas-limits", "--skip", "test", "--skip", "script"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[PASS] Vault.credit(address,uint256): gas 47174 <= @custom:max-gas 60000 (worst case replays at 47174)
[FAIL] Vault.creditTight(address,uint256): gas 47130 > @custom:max-gas 40000 (worst case replays at 47130); sender=[..] calldata=[..]
[PASS] Vault.setFee(uint256): gas 46304 <= @custom:max-gas 50000 (worst case replays at 46304)
[INCOMPLETE] Vault.withdraw(uint256): unsupported symbolic execution feature: external call to unknown code has no gas bound
  hint: bound the callee in forge test --symbolic, where setUp deploys it, or give the call an explicit gas limit

See https://github.com/foundry-rs/foundry/blob/master/crates/evm/symbolic/docs/gas-bounds-guide.md for how to write gas checks that finish.

"#]])
        .stderr_eq(str![[r#"
...
Error: 2 of 4 gas annotation checks are not proven
...
"#]]);
}

// `@custom:max-length` bounds a dynamic parameter, so the build check covers every length up to
// it. A dynamic parameter without one, or a length above the exhaustive cap, is incomplete.
#[forgetest_init]
fn build_checks_gas_limits_with_max_length(prj: _, cmd: _) {
    skip_unless_z3!("build_checks_gas_limits_with_max_length");

    prj.add_source(
        "Airdrop.sol",
        r#"
contract Airdrop {
    mapping(address => uint256) public credited;

    /// @custom:max-gas 300_000
    /// @custom:max-length recipients 2
    function airdrop(address[] calldata recipients, uint256 amount) external {
        for (uint256 i; i < recipients.length; ++i) {
            credited[recipients[i]] += amount;
        }
    }

    /// @custom:max-gas 300_000
    function noLength(address[] calldata recipients) external {
        for (uint256 i; i < recipients.length; ++i) credited[recipients[i]] += 1;
    }

    /// @custom:max-gas 30_000_000
    /// @custom:max-length recipients 500
    function tooLong(address[] calldata recipients) external {
        for (uint256 i; i < recipients.length; ++i) credited[recipients[i]] += 1;
    }
}
"#,
    );

    prj.add_source(
        "Gate.sol",
        r#"
contract Gate {
    address signer;
    mapping(bytes32 => bool) used;

    /// @custom:max-gas 100_000
    /// @custom:max-length signature 65
    function consume(bytes32 digest, bytes calldata signature) external {
        require(!used[digest] && signature.length == 65);
        // A known invalid recovery ID makes the replay independent of the opaque signer model.
        require(uint8(signature[64]) == 1);
        bytes32 r = bytes32(signature[0:32]);
        bytes32 s = bytes32(signature[32:64]);
        require(ecrecover(digest, uint8(signature[64]), r, s) == signer);
        used[digest] = true;
    }

    /// @custom:max-gas 100_000
    function consumeFromContract(bytes32 digest) external {
        require(signer.code.length > 0);
        used[digest] = true;
    }
}
"#,
    );

    cmd.args(["build", "--gas-limits", "--skip", "test", "--skip", "script"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[PASS] Airdrop.airdrop(address[],uint256): gas 68884 <= @custom:max-gas 300000 (worst case replays at 68884)
[INCOMPLETE] Airdrop.noLength(address[]): dynamic parameter `recipients` needs a @custom:max-length
  hint: add `@custom:max-length <parameter> <N>`
[INCOMPLETE] Airdrop.tooLong(address[]): @custom:max-length 500 for `recipients` is above 16; larger parameter lengths are not supported yet
[PASS] Gate.consume(bytes32,bytes): gas 52870 <= @custom:max-gas 100000 (worst case replays at 52870)
[PASS] Gate.consumeFromContract(bytes32): gas 48959 <= @custom:max-gas 100000 (worst case did not replay)

See https://github.com/foundry-rs/foundry/blob/master/crates/evm/symbolic/docs/gas-bounds-guide.md for how to write gas checks that finish.

"#]])
        .stderr_eq(str![[r#"
...
Error: 2 of 5 gas annotation checks are not proven
...
"#]]);
}

// `@custom:max-length` on a storage array bounds the loops over it, and every function that
// writes storage must keep the array within its length.
#[forgetest_init]
fn build_checks_gas_limits_with_storage_loops(prj: _, cmd: _) {
    skip_unless_z3!("build_checks_gas_limits_with_storage_loops");

    prj.add_source(
        "Payroll.sol",
        r#"
contract Payroll {
    /// @custom:max-length 500
    address[] employees;
    mapping(address => uint256) owed;
    uint256 head;
    uint256 tail;

    /// @custom:max-gas 5_000_000
    function settle() external {
        for (uint256 i; i < employees.length; ++i) owed[employees[i]] = 0;
    }

    /// @custom:max-gas 3_000_000
    function findIdle() external view returns (uint256 i) {
        for (; i < employees.length; ++i) {
            if (owed[employees[i]] == 0) break;
        }
    }

    /// @custom:max-gas 50_000
    function settleTight() external {
        for (uint256 i; i < employees.length; ++i) owed[employees[i]] = 0;
    }

    function hire(address who) external {
        employees.push(who);
    }

    function hireChecked(address who) external {
        require(employees.length < 500);
        employees.push(who);
    }

    function fire() external {
        employees.pop();
    }

    /// @custom:max-gas 30_000_000
    function drain() external {
        while (head < tail) head++;
    }
}
"#,
    );

    cmd.args(["build", "--gas-limits", "--skip", "test", "--skip", "script"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[INCOMPLETE] Payroll.drain(): unsupported symbolic execution feature: loop without an iteration bound; bound its array with @custom:max-length; worst found so far: gas 25434, replays at 25434
[PASS] Payroll.findIdle(): gas 2369112 <= @custom:max-gas 3000000 (loop at pc 884: 499 more iterations of <= 4682 gas, then <= 4846) (witness replays at 28112)
[PASS] Payroll.fire(): keeps every @custom:max-length
[FAIL] Payroll.hire(address): `employees` can grow to 501 > @custom:max-length 500; sender=0x0000000000000000000000000000000000000000 calldata=0xd9d6bd860000000000000000000000008080808080808080808080808080808080804080
[PASS] Payroll.hireChecked(address): keeps every @custom:max-length
[PASS] Payroll.settle(): gas 3808868 <= @custom:max-gas 5000000 (loop at pc 370: 499 more iterations of <= 7571 gas, then <= 145) (witness replays at 1360968)
[FAIL] Payroll.settleTight(): gas 3808867 > @custom:max-gas 50000 (loop at pc 634: 499 more iterations of <= 7571 gas, then <= 145) (witness replays at 1360967); sender=0x0000000000000000000000000000000000000000 calldata=0xb56e4d95

"#]])
        .stderr_eq(str![[r#"
...
Error: 3 of 7 gas annotation checks are not proven
...
"#]]);
}

// A check that stops early still fails on an input that replays above the limit, and otherwise
// reports the worst gas it found with a hint.
#[forgetest_init]
fn build_checks_gas_limits_report_finished_paths(prj: _, cmd: _) {
    skip_unless_z3!("build_checks_gas_limits_report_finished_paths");

    prj.update_config(|config| config.symbolic.max_paths = 2);
    prj.add_source(
        "Modes.sol",
        r#"
contract Modes {
    uint256 a;
    uint256 b;
    uint256 c;

    /// @custom:max-gas 30_000
    function set(uint8 mode, uint256 v) external {
        if (mode == 0) a = v;
        else if (mode == 1) b = v;
        else if (mode == 2) c = v;
        else (a, b, c) = (v, v, v);
    }

    /// @custom:max-gas 1_000_000
    function setLoose(uint8 mode, uint256 v) external {
        if (mode == 0) a = v;
        else if (mode == 1) b = v;
        else if (mode == 2) c = v;
        else (a, b, c) = (v, v, v);
    }
}
"#,
    );

    cmd.args(["build", "--gas-limits", "--skip", "test", "--skip", "script"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL] Modes.set(uint8,uint256): an input replays at 44454 > @custom:max-gas 30000, found before the search stopped (symbolic path limit exceeded (2)); sender=0x0000000000000000000000000000000000000000 calldata=0x48ab5e6c0000000000000000000000000000000000000000000000000000000000000001fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd
[INCOMPLETE] Modes.setLoose(uint8,uint256): symbolic path limit exceeded (2); worst found so far: gas 44476, replays at 44476
  hint: check one mode at a time and fix mapping keys you do not care about

See https://github.com/foundry-rs/foundry/blob/master/crates/evm/symbolic/docs/gas-bounds-guide.md for how to write gas checks that finish.

"#]])
        .stderr_eq(str![[r#"
...
Error: 2 of 2 gas annotation checks are not proven
...
"#]]);
}

#[forgetest_init]
fn symbolic_gas_rejects_unsupported_annotations(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_gas_rejects_unsupported_annotations");
    prj.add_source(
        "InvalidGas.sol",
        r#"
contract InvalidGas {
    uint256 stored;
    /// @custom:max-gas typo
    function badGas() external { stored = 42; }
    /// @custom:max-gas 30000
    /// @custom:max-length data nope
    function badLength(bytes calldata data) external { stored = data.length; }
    /// @custom:max-gas 30000
    /// @custom:max-length data
    function badEntry(bytes calldata data) external { stored = data.length; }
    /// @custom:max-gas 30000
    function deposit() external payable { if (msg.value != 0) stored = 42; }
    /// @custom:max-gas 30000
    fallback() external { stored = 42; }
    /// @custom:max-gas 30000
    receive() external payable { stored = 42; }
}
contract StorageFallback {
    /// @custom:max-length 0
    uint256[] values;
    fallback() external { values.push(42); }
}
"#,
    );
    prj.add_test(
        "InvalidGas.t.sol",
        r#"
import "../src/InvalidGas.sol";
contract InvalidGasTest {
    InvalidGas target;
    function setUp() public { target = new InvalidGas(); }
    function testReady() public view { require(address(target) != address(0)); }
}
"#,
    );
    cmd.forge_fuse().args(["build", "--gas-limits", "--skip", "test", "--skip", "script", "--json"])
        .assert_json_stdout_with_status(false, str![[r#"
[
  {"contract":"InvalidGas","signature":"fallback()","status":"incomplete","reason":"gas and storage-length checks for fallback functions are not supported"},
  {"contract":"InvalidGas","signature":"receive()","status":"incomplete","reason":"gas and storage-length checks for receive functions are not supported"},
  {"contract":"InvalidGas","signature":"badEntry(bytes)","status":"incomplete","reason":"invalid @custom:max-length `data`"},
  {"contract":"InvalidGas","signature":"badGas()","status":"incomplete","reason":"invalid @custom:max-gas `typo`"},
  {"contract":"InvalidGas","signature":"badLength(bytes)","status":"incomplete","reason":"invalid @custom:max-length `data nope`"},
  {"contract":"InvalidGas","signature":"deposit()","limit":30000,"status":"incomplete","reason":"gas bounds for payable functions require symbolic transaction value"},
  {"contract":"StorageFallback","signature":"fallback()","status":"incomplete","reason":"gas and storage-length checks for fallback functions are not supported"}
]
"#]]);
    assert_symbolic(cmd.forge_fuse().args(["test", "--symbolic", "--match-contract", "InvalidGasTest"]))
        .failure().stdout_eq(str![[r#"
...
Ran 7 tests for test/InvalidGas.t.sol:InvalidGasTest
[FAIL: incomplete symbolic execution (Stuck): invalid @custom:max-length `data`] maxGas:InvalidGas.badEntry(bytes) ([METRICS])
[FAIL: incomplete symbolic execution (Stuck): invalid @custom:max-gas `typo`] maxGas:InvalidGas.badGas() ([METRICS])
[FAIL: incomplete symbolic execution (Stuck): invalid @custom:max-length `data nope`] maxGas:InvalidGas.badLength(bytes) ([METRICS])
[FAIL: incomplete symbolic execution (Stuck): gas bounds for payable functions require symbolic transaction value] maxGas:InvalidGas.deposit() ([METRICS])
[FAIL: incomplete symbolic execution (Stuck): gas bounds for fallback functions are not supported] maxGas:InvalidGas.fallback() ([METRICS])
[FAIL: incomplete symbolic execution (Stuck): gas bounds for receive functions are not supported] maxGas:InvalidGas.receive() ([METRICS])
[PASS] testReady() ([GAS])
...
"#]]);
}
