//! Contains various tests for `forge test` with precompiles.

use foundry_evm_networks::NetworkConfigs;
use foundry_test_utils::str;

// tests transfer using celo precompile.
// <https://github.com/foundry-rs/foundry/issues/11622>
forgetest_init!(celo_transfer, |prj, cmd| {
    prj.update_config(|config| {
        config.networks = NetworkConfigs::with_celo();
    });

    prj.add_test(
        "CeloTransfer.t.sol",
        r#"
import "forge-std/Test.sol";

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
}

contract CeloTransferTest is Test {
    IERC20 celo = IERC20(0x471EcE3750Da237f93B8E339c536989b8978a438);
    IERC20 usdc = IERC20(0xcebA9300f2b948710d2653dD7B07f33A8B32118C);
    IERC20 usdt = IERC20(0x48065fbBE25f71C9282ddf5e1cD6D6A887483D5e);

    address binanceAccount = 0xf6436829Cf96EA0f8BC49d300c536FCC4f84C4ED;
    address recipient = makeAddr("recipient");

    function setUp() public {
        vm.createSelectFork("https://forno.celo.org");
    }

    function testCeloBalance() external {
        console2.log("recipient balance before", celo.balanceOf(recipient));
        vm.prank(binanceAccount);
        celo.transfer(recipient, 100);
        console2.log("recipient balance after", celo.balanceOf(recipient));
        assertEq(celo.balanceOf(recipient), 100);
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "testCeloBalance", "-vvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/CeloTransfer.t.sol:CeloTransferTest
[PASS] testCeloBalance() ([GAS])
Logs:
  recipient balance before 0
  recipient balance after 100

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});
