use foundry_compilers::artifacts::EvmVersion;

forgetest!(can_translate_balances_after_switch_to_pvm, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "BalanceTranslationTest.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {console} from "./console.sol";

contract BalanceTranslationTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_BalanceTranslationRevmPvm() public {
        uint256 amount = 10 ether;
        vm.deal(address(this), amount);

        uint256 initialBalance = address(this).balance;
        assertEq(initialBalance, amount);

        vm.pvm(true);

        uint256 currentBalance = address(this).balance;
        console.log(initialBalance, currentBalance);
        assertEq(initialBalance, currentBalance);
    }
}
"#,
    )
    .unwrap();
    prj.update_config(|config| config.evm_version = EvmVersion::Cancun);

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/BalanceTranslationTest.t.sol:BalanceTranslationTest
[PASS] test_BalanceTranslationRevmPvm() ([GAS])
Logs:
  10000000000000000000 10000000000000000000

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(counter_test, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "Counter.sol",
        r#"
    // SPDX-License-Identifier: UNLICENSED
    pragma solidity ^0.8.13;

    contract Counter {
        uint256 public number = 0;

        function setNumber(uint256 newNumber) public {
            number = newNumber;
        }

        function increment() public {
            number = number + 1;
        }
    }
    "#,
    )
    .unwrap();
    prj.add_source(
        "CounterTest.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {Counter} from "./Counter.sol";
import {console} from "./console.sol";

contract CounterTest is DSTest {
  Vm constant vm = Vm(HEVM_ADDRESS);
  Counter public counter;

  function setUp() public {
    vm.pvm(true);
    counter = new Counter(); 
    counter.setNumber(5);
    assertEq(counter.number(), 5);
  }

  function test_Increment() public {
      assertEq(counter.number(), 5);
      counter.setNumber(55); 
      assertEq(counter.number(), 55);
      counter.increment(); 
      assertEq(counter.number(), 56);
  }

  function testFuzz_SetNumber(uint256 x) public {
      assertEq(counter.number(), 5);
      counter.setNumber(x); 
      assertEq(counter.number(), x);
  }
  
  function testFuzz_SetNumber2(uint256 x) public {
    assertEq(counter.number(), 5);
    counter.setNumber(x); 
    assertEq(counter.number(), x);
  }

  function testFuzz_SetNumber3(uint256 x) public {
    assertEq(counter.number(), 5);
    counter.setNumber(x); 
    assertEq(counter.number(), x);
  }
}
"#,
    )
    .unwrap();
    prj.update_config(|config| config.evm_version = EvmVersion::Cancun);

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 4 tests for src/CounterTest.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumber2(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumber3(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]);
});

forgetest!(set_get_nonce_revive, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "SetNonce.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {console} from "./console.sol";

contract SetNonce is DSTest {
  Vm constant vm = Vm(HEVM_ADDRESS);

  function test_SetNonce() public {
      vm.pvm(true);
      uint64 original = vm.getNonce(address(this));
      vm.setNonce(address(this), 64);
      uint64 newValue = vm.getNonce(address(this));
      assert(original != newValue);
      assertEq(newValue, 64);
  }
}
"#,
    )
    .unwrap();
    prj.update_config(|config| config.evm_version = EvmVersion::Cancun);

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/SetNonce.t.sol:SetNonce
[PASS] test_SetNonce() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(roll_revive, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "Roll.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {console} from "./console.sol";

contract Roll is DSTest {
  Vm constant vm = Vm(HEVM_ADDRESS);

  function test_Roll() public {
      vm.pvm(true);
      uint256 original = block.number;
      vm.roll(10);
      uint256 newValue = block.number;
      assert(original != newValue);
      assertEq(newValue, 10);
  }
}
"#,
    )
    .unwrap();

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/Roll.t.sol:Roll
[PASS] test_Roll() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(warp_revive, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "Warp.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {console} from "./console.sol";

contract Warp is DSTest {
  Vm constant vm = Vm(HEVM_ADDRESS);

  function test_Warp() public {
      vm.pvm(true);
      uint256 original = block.timestamp;
      vm.warp(100);
      uint256 newValue = block.timestamp;
      assert(original != newValue);
      assertEq(newValue, 100);
  }
}
"#,
    )
    .unwrap();

    cmd.env("RUST_LOG", "revive_strategy");
    let res = cmd.args(["test", "--resolc", "-vvv", "--resolc-startup"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!
[..] INFO revive_strategy::cheatcodes: startup PVM migration initiated
[..] INFO revive_strategy::cheatcodes: switching to PVM
[..] INFO revive_strategy::cheatcodes: startup PVM migration completed
[..] INFO revive_strategy::cheatcodes: cheatcode=pvmCall { enabled: true } using_pvm=true
[..] INFO revive_strategy::cheatcodes: already in PVM
[..] INFO revive_strategy::cheatcodes: cheatcode=warpCall { newTimestamp: 100 } using_pvm=true

Ran 1 test for src/Warp.t.sol:Warp
[PASS] test_Warp() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(deal, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "Balance.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {console} from "./console.sol";

contract Balance is DSTest {
Vm constant vm = Vm(HEVM_ADDRESS);

function test_Balance() public {
  vm.deal(address(this), 64 ether);
  uint256 newValue = address(this).balance;
  assertEq(newValue, 64 ether);
  vm.deal(address(this), 65 ether);
  uint256 newValue2 = address(this).balance;
  assertEq(newValue2, 65 ether);
}
}
"#,
    )
    .unwrap();

    cmd.env("RUST_LOG", "revive_strategy");
    let res = cmd.args(["test", "--resolc", "-vvv", "--resolc-startup"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!
[..] INFO revive_strategy::cheatcodes: startup PVM migration initiated
[..] INFO revive_strategy::cheatcodes: switching to PVM
[..] INFO revive_strategy::cheatcodes: startup PVM migration completed
[..] INFO revive_strategy::cheatcodes: cheatcode=dealCall { account: 0x7fa9385be102ac3eac297483dd6233d62b3e1496, newBalance: 64000000000000000000 } using_pvm=true
[..] INFO revive_strategy::cheatcodes: operation="get_balance" using_pvm=true target=0x7fa9385be102ac3eac297483dd6233d62b3e1496 balance=64000000000000000000
[..] INFO revive_strategy::cheatcodes: cheatcode=dealCall { account: 0x7fa9385be102ac3eac297483dd6233d62b3e1496, newBalance: 65000000000000000000 } using_pvm=true
[..] INFO revive_strategy::cheatcodes: operation="get_balance" using_pvm=true target=0x7fa9385be102ac3eac297483dd6233d62b3e1496 balance=65000000000000000000

Ran 1 test for src/Balance.t.sol:Balance
[PASS] test_Balance() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(vm_load, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.insert_console();
    prj.add_source(
        "Counter.sol",
        r#"
  // SPDX-License-Identifier: UNLICENSED
  pragma solidity ^0.8.13;

  contract Counter {
      uint256 public number;

      constructor (uint256 number_) {
        number = number_;
      }
  }
  "#,
    )
    .unwrap();
    prj.add_source(
        "Load.t.sol",
        r#"
import "./test.sol";
import "./Vm.sol";
import {console} from "./console.sol";
import {Counter} from "./Counter.sol";

contract Load is DSTest {
Vm constant vm = Vm(HEVM_ADDRESS);

function testFuzz_Load(uint256 x) public {
    vm.pvm(true);
    Counter counter = new Counter(x);
    bytes32 res = vm.load(address(counter), bytes32(uint256(0)));
    assertEq(uint256(res), x);
}
}
"#,
    )
    .unwrap();

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/Load.t.sol:Load
[PASS] testFuzz_Load(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});
