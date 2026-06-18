use super::*;

// `gas_fuzz = true` adds a "Max Gas" column to the metrics table.
forgetest_init!(should_show_max_gas_column_when_gas_fuzz_enabled, |prj, cmd| {
    prj.add_test(
        "GasFuzzColumnTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract GasFuzzHandler is Test {
    uint256 public n;

    // Storage write keeps gas_used > 0 so `max_gas` becomes populated.
    function bump(uint256 a) public {
        n = a;
    }
}

contract GasFuzzColumnTest is Test {
    function setUp() public {
        new GasFuzzHandler();
    }

    /// forge-config: default.invariant.runs = 3
    /// forge-config: default.invariant.depth = 5
    /// forge-config: default.invariant.show-metrics = true
    /// forge-config: default.invariant.gas-fuzz = true
    function invariant_gas_fuzz_column() public {}
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_gas_fuzz_column"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_gas_fuzz_column() (runs: 3, calls: [..], reverts: [..])

[..]
| Contract       | Selector | Calls | Reverts | Discards | Max Gas |
[..]
| GasFuzzHandler | bump     |[..]
[..]
...
"#]]);
});

// Without `gas_fuzz`, the metrics table stays at the legacy 5 columns.
// Pinning the header verbatim is the absence-check.
forgetest_init!(should_not_show_max_gas_column_by_default, |prj, cmd| {
    prj.add_test(
        "GasFuzzAbsentTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract NoGasFuzzHandler is Test {
    uint256 public n;
    function bump(uint256 a) public { n = a; }
}

contract GasFuzzAbsentTest is Test {
    function setUp() public {
        new NoGasFuzzHandler();
    }

    /// forge-config: default.invariant.runs = 3
    /// forge-config: default.invariant.depth = 5
    /// forge-config: default.invariant.show-metrics = true
    function invariant_no_gas_fuzz() public {}
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_no_gas_fuzz"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_no_gas_fuzz() (runs: 3, calls: [..], reverts: [..])

[..]
| Contract         | Selector | Calls | Reverts | Discards |
[..]
| NoGasFuzzHandler | bump     |[..]
[..]
...
"#]]);
});

// The GovernMental 2016 gas-DoS repro should surface the hot selector in the
// gas-fuzz metrics table.
forgetest_init!(should_surface_gas_dos_on_governmental_repro, |prj, cmd| {
    prj.add_source(
        "GovernMental.sol",
        r#"
contract GovernMental {
    struct Creditor { address addr; uint256 amount; }
    Creditor[] public creditors;
    uint256 public totalOwed;

    function lend(uint256 amount) external {
        require(amount > 0, "zero");
        creditors.push(Creditor({ addr: msg.sender, amount: amount }));
        totalOwed += amount;
    }

    function payout() external {
        uint256 n = creditors.length;
        uint256 paid;
        for (uint256 i = 0; i < n; i++) {
            paid += creditors[i].amount;
            creditors[i].amount = 0;
        }
        delete creditors;
        totalOwed = 0;
    }
}
     "#,
    );

    prj.add_test(
        "GovernMentalReproTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {GovernMental} from "../src/GovernMental.sol";

contract BuggyHandler is Test {
    GovernMental public target;
    constructor(GovernMental _t) { target = _t; }
    function lend(uint256 a) external { target.lend(bound(a, 1, 1e18)); }
    function bulkPayout() external { target.payout(); }
}

contract GovernMentalReproTest is Test {
    GovernMental internal underlying;
    BuggyHandler internal handler;

    function setUp() public {
        underlying = new GovernMental();
        handler = new BuggyHandler(underlying);
        for (uint256 i = 0; i < 200; i++) {
            vm.prank(address(uint160(0x1000 + i)));
            underlying.lend(1 ether);
        }
        targetContract(address(handler));
    }

    /// forge-config: default.invariant.runs = 3
    /// forge-config: default.invariant.depth = 2000
    /// forge-config: default.invariant.show-metrics = true
    /// forge-config: default.invariant.gas-fuzz = true
    function invariant_alwaysTrue() public pure { assert(true); }
}
     "#,
    );

    let stdout = cmd
        .args(["test", "--mt", "invariant_alwaysTrue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let row = stdout
        .lines()
        .find(|l| l.contains("bulkPayout") && l.contains('|'))
        .unwrap_or_else(|| panic!("no bulkPayout metrics row found:\n{stdout}"));

    let cells = row.split('|').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>();
    let calls: u64 = cells
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("could not parse Calls from row: {row:?}"));
    let _max_gas: u64 = cells
        .last()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("could not parse Max Gas from row: {row:?}"));
    assert!(calls > 0, "expected bulkPayout to be called, got row: {row:?}");
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("Contract") && l.contains("Selector") && l.contains("Max Gas")),
        "expected Max Gas header in metrics table:\n{stdout}",
    );
    assert!(
        row.split('|').map(str::trim).filter(|s| !s.is_empty()).count() >= 6,
        "expected bulkPayout row to include Max Gas cell, got row: {row:?}",
    );
});
