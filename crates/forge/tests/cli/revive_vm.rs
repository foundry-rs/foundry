// TODO: False positive, after switch to PVM we still read balance from EVM
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
