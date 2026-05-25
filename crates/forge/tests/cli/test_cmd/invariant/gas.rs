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

// The GovernMental 2016 gas-DoS bug should surface as `bulkPayout` Max
// Gas > 500k (clears the bounded-fixed ~474k cap with margin).
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

    // Last `|`-delimited cell of the `bulkPayout` row is its Max Gas value.
    let row = stdout
        .lines()
        .find(|l| l.contains("bulkPayout") && l.contains('|'))
        .unwrap_or_else(|| panic!("no bulkPayout metrics row found:\n{stdout}"));
    let max_gas: u64 = row
        .split('|')
        .map(str::trim)
        .rfind(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("could not parse Max Gas from row: {row:?}"));
    assert!(
        max_gas > 500_000,
        "expected bulkPayout Max Gas > 500_000 (gas DoS), got {max_gas} from row: {row:?}",
    );
});

// Razor gas surfaces a swallowed inner-OOG state-corruption bug: the inner
// SSTORE loop OOGs, the outer keeps its retained 1/64 and silently advances
// `debited` without the matching `credited` write.
const SWALLOWED_OOG_SOURCE: &str = r#"
contract Inner {
    uint256 public credited;
    uint256[] public log;

    function credit(uint256 amount) external {
        credited += amount;
        for (uint256 i = 0; i < 80; i++) {
            log.push(i);
        }
    }
}

contract Outer {
    Inner public inner;
    uint256 public debited;

    constructor() {
        inner = new Inner();
    }

    function transfer(uint256 amount) external {
        debited += amount;
        (bool ok, ) = address(inner).call(
            abi.encodeWithSelector(Inner.credit.selector, amount)
        );
        ok; // swallowed failure (the bug)
    }
}
"#;

const SWALLOWED_OOG_HANDLER: &str = r#"
import {Test} from "forge-std/Test.sol";
import {Outer, Inner} from "../src/SwallowedOOG.sol";

contract SwallowedOOGHandler is Test {
    Outer public outer;
    constructor(Outer _o) { outer = _o; }
    function doTransfer(uint256 amount) external {
        outer.transfer(bound(amount, 1, 1e9));
    }
}
"#;

forgetest_init!(should_surface_swallowed_oog_corruption_under_gas_fuzz, |prj, cmd| {
    prj.add_source("SwallowedOOG.sol", SWALLOWED_OOG_SOURCE);
    prj.add_test(
        "SwallowedOOGUnderGasFuzz.t.sol",
        &format!(
            r#"
{SWALLOWED_OOG_HANDLER}

contract SwallowedOOGUnderGasFuzzTest is Test {{
    Outer public outer;
    SwallowedOOGHandler public handler;

    function setUp() public {{
        outer = new Outer();
        handler = new SwallowedOOGHandler(outer);
        targetContract(address(handler));
    }}

    /// forge-config: default.invariant.runs = 16
    /// forge-config: default.invariant.depth = 200
    /// forge-config: default.invariant.fail-on-revert = false
    /// forge-config: default.invariant.gas-fuzz = true
    function invariant_oog_debitedEqualsCredited() public view {{
        assertEq(
            outer.debited(),
            outer.inner().credited(),
            "swallowed OOG corruption"
        );
    }}
}}
         "#
        ),
    );

    cmd.args(["test", "--mt", "invariant_oog_debitedEqualsCredited"]).assert_failure().stdout_eq(
        str![[r#"
...
[FAIL: swallowed OOG corruption: [..]]
...
 invariant_oog_debitedEqualsCredited() (runs: [..], calls: [..], reverts: [..])
...
"#]],
    );
});

// Control: without gas-fuzz the natural ~30M gas always covers the inner;
// the same harness, depth, and seed never reach the razor and the invariant
// holds.
forgetest_init!(should_not_surface_swallowed_oog_corruption_without_gas_fuzz, |prj, cmd| {
    prj.add_source("SwallowedOOG.sol", SWALLOWED_OOG_SOURCE);
    prj.add_test(
        "SwallowedOOGWithoutGasFuzz.t.sol",
        &format!(
            r#"
{SWALLOWED_OOG_HANDLER}

contract SwallowedOOGWithoutGasFuzzTest is Test {{
    Outer public outer;
    SwallowedOOGHandler public handler;

    function setUp() public {{
        outer = new Outer();
        handler = new SwallowedOOGHandler(outer);
        targetContract(address(handler));
    }}

    /// forge-config: default.invariant.runs = 16
    /// forge-config: default.invariant.depth = 200
    /// forge-config: default.invariant.fail-on-revert = false
    /// forge-config: default.invariant.gas-fuzz = false
    function invariant_oog_natural_gas_passes() public view {{
        assertEq(
            outer.debited(),
            outer.inner().credited(),
            "swallowed OOG corruption"
        );
    }}
}}
         "#
        ),
    );

    cmd.args(["test", "--mt", "invariant_oog_natural_gas_passes"]).assert_success().stdout_eq(
        str![[r#"
...
[PASS] invariant_oog_natural_gas_passes() (runs: 16, calls: [..], reverts: [..])
...
"#]],
    );
});

// Per-call `tx.gasprice` actually reaches the handler under `gas_fuzz = true`.
// The handler records distinct `tx.gasprice` values into storage; the invariant
// asserts no diversity, so it must fire once the sampler delivers a second
// distinct value. If the sampled price were written to the wrong executor (the
// pre-fix bug) `uniqueCount` would stay at 1 and the invariant would never fire.
forgetest_init!(should_apply_sampled_gas_price_to_call, |prj, cmd| {
    prj.add_test(
        "GasPriceFuzzTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract GasPriceObserver {
    mapping(uint256 => bool) public seen;
    uint256 public uniqueCount;

    function probe() external {
        uint256 p = tx.gasprice;
        if (!seen[p]) {
            seen[p] = true;
            uniqueCount++;
        }
    }
}

contract GasPriceFuzzTest is Test {
    GasPriceObserver public obs;

    function setUp() public {
        obs = new GasPriceObserver();
        targetContract(address(obs));
    }

    /// forge-config: default.invariant.runs = 2
    /// forge-config: default.invariant.depth = 20
    /// forge-config: default.invariant.fail-on-revert = false
    /// forge-config: default.invariant.gas-fuzz = true
    function invariant_gasPriceStaysConstant() public view {
        assertLe(obs.uniqueCount(), 1, "gas-price diversity observed");
    }
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_gasPriceStaysConstant"]).assert_failure().stdout_eq(str![
        [r#"
...
[FAIL: gas-price diversity observed[..]]
...
 invariant_gasPriceStaysConstant() (runs: [..], calls: [..], reverts: [..])
...
"#]
    ]);
});

forgetest_init!(should_not_leak_sampled_gas_price_to_invariant_call, |prj, cmd| {
    prj.add_test(
        "GasPriceIsolationTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract GasPriceIsolationObserver {
    mapping(uint256 => bool) public seen;
    uint256 public uniqueCount;

    function probe() external {
        uint256 p = tx.gasprice;
        if (!seen[p]) {
            seen[p] = true;
            uniqueCount++;
        }
    }
}

contract GasPriceIsolationTest is Test {
    GasPriceIsolationObserver public obs;

    function setUp() public {
        obs = new GasPriceIsolationObserver();
        targetContract(address(obs));
    }

    /// forge-config: default.invariant.runs = 2
    /// forge-config: default.invariant.depth = 20
    /// forge-config: default.invariant.fail-on-revert = false
    /// forge-config: default.invariant.gas-fuzz = true
    function invariant_sampledGasPriceIsHandlerOnly() public view {
        assertEq(tx.gasprice, 0, "sampled gas price leaked");
    }

    function afterInvariant() public view {
        assertGt(obs.uniqueCount(), 1, "handler saw sampled prices");
    }
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_sampledGasPriceIsHandlerOnly"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] invariant_sampledGasPriceIsHandlerOnly() (runs: 2, calls: [..], reverts: [..])
...
"#]]);
});

forgetest_init!(should_replay_persisted_gas_price_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 2;
        config.invariant.depth = 20;
        config.invariant.fail_on_revert = false;
        config.invariant.gas_fuzz = true;
    });
    prj.add_test(
        "GasPricePersistedReplayTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract PersistedGasPriceObserver {
    mapping(uint256 => bool) public seen;
    uint256 public uniqueCount;

    function probe() external {
        uint256 p = tx.gasprice;
        if (!seen[p]) {
            seen[p] = true;
            uniqueCount++;
        }
    }
}

contract GasPricePersistedReplayTest is Test {
    PersistedGasPriceObserver public obs;

    function setUp() public {
        obs = new PersistedGasPriceObserver();
        targetContract(address(obs));
    }

    function invariant_gasPriceStaysConstant() public view {
        assertLe(obs.uniqueCount(), 1, "gas-price diversity observed");
    }
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_gasPriceStaysConstant"]).assert_failure();

    let persisted = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("GasPricePersistedReplayTest")
        .join("invariants")
        .join("invariant_gasPriceStaysConstant");
    let json: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(&persisted).unwrap()).unwrap();
    let calls = json["call_sequence"].as_array().unwrap();
    assert!(calls.iter().any(|call| call.get("gas_price").is_some()));

    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    cmd.forge_fuse()
        .args(["test", "--mt", "invariant_gasPriceStaysConstant"])
        .assert_failure()
        .stderr_eq(str![["
...
Warning: Replayed invariant failure from persisted file.
Run `forge clean` or remove file to ignore failure and to continue invariant test campaign.
...
"]]);
});
