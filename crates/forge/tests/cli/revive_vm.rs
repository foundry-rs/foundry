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

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert_success();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
usually needed in the following cases:
  1. To detect whether an address belongs to a smart contract.
  2. To detect whether the deploy code execution has finished.
Polkadot comes with native account abstraction support (so smart contracts are just accounts
coverned by code), and you should avoid differentiating between contracts and non-contract
addresses.
[FILE]

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

    let res = cmd.args(["test", "--resolc", "-vvv"]).assert();
    res.stderr_eq(str![""]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
usually needed in the following cases:
  1. To detect whether an address belongs to a smart contract.
  2. To detect whether the deploy code execution has finished.
Polkadot comes with native account abstraction support (so smart contracts are just accounts
coverned by code), and you should avoid differentiating between contracts and non-contract
addresses.
[FILE]

Ran 4 tests for src/CounterTest.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumber2(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumber3(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]);
});
