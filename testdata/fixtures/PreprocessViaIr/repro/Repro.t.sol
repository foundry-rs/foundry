// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

interface Vm {
    function envOr(string calldata name, string calldata def) external returns (string memory);
    function createSelectFork(string calldata rpc) external returns (uint256);
    function startPrank(address a) external;
    function prank(address a) external;
    function stopPrank() external;
    function expectRevert(bytes4 e) external;
    function assertGt(uint256 a, uint256 b) external pure;
    function assertEq(uint256 a, uint256 b) external pure;
}
import {Empty, HedgerPool} from "../src/core/HedgerPool.sol";

interface IERC20 {
    function balanceOf(address a) external view returns (uint256);
}

library HedgerPoolErrorLibrary {
    error PositionClosureRestricted();
}

abstract contract Test {
    Vm internal constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    event log(string);

    function assertGt(uint256 a, uint256 b) internal pure {
        if (a <= b) vm.assertGt(a, b);
    }

    function assertEq(uint256 a, uint256 b) internal pure {
        if (a != b) vm.assertEq(a, b);
    }
}

contract HedgerPoolDustDeadlockForkTest is Test {
    address internal constant HEDGER_POOL = 0xff5D7cE5c7671B2EA805Ee752B4f8eC9Ecf2975A;
    address internal constant HEDGER = 0x8DAD1B6c1A40e2649d50952977b5af1992f098d1;
    address internal constant TIMELOCK = 0x7Ade8f3Bf1FdaF0785efE9Ea5C6339D1aD6B8342;
    address internal constant TIME_PROVIDER = 0x520236487CBD0a6958B4EefC7853cd7C3F5C56E7;
    address internal constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    uint256 internal constant POSITION_ID = 1;

    function testLiveDustDeadlockResolvedByUpgrade() public {
        string memory rpc = vm.envOr("BASE_FORK_RPC_URL", string(""));
        if (bytes(rpc).length == 0) {
            emit log("SKIP: set BASE_FORK_RPC_URL to run this fork validation");
            return;
        }
        vm.createSelectFork(rpc);

        HedgerPool pool = HedgerPool(HEDGER_POOL);

        vm.startPrank(HEDGER);
        vm.expectRevert(HedgerPoolErrorLibrary.PositionClosureRestricted.selector);
        pool.exitHedgePosition(POSITION_ID);
        vm.stopPrank();

        HedgerPool newImpl = new HedgerPool(TIME_PROVIDER);
        vm.prank(TIMELOCK);
        pool.upgradeToAndCall(address(newImpl), "");

        (,,, uint96 marginBefore,,,,,,,,,) = pool.positions(POSITION_ID);
        assertGt(uint256(marginBefore), 0);
        uint256 hedgerUsdcBefore = IERC20(USDC).balanceOf(HEDGER);

        vm.startPrank(HEDGER);
        pool.exitHedgePosition(POSITION_ID);
        vm.stopPrank();

        assertEq(IERC20(USDC).balanceOf(HEDGER) - hedgerUsdcBefore, uint256(marginBefore));
        assertEq(pool.totalMargin(), 0);
    }
}

contract DeploymentSmokeTest {
    function test_deployment() public {
        HedgerPool target = new HedgerPool(address(42));
        require(target.totalMargin() == 42, "constructor argument");

        bytes32 salt = bytes32(uint256(1));
        Empty empty = new Empty{value: 1, salt: salt}();
        address expected = address(
            uint160(
                uint256(
                    keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, keccak256(type(Empty).creationCode)))
                )
            )
        );
        require(address(empty) == expected, "deployment salt");
        require(address(empty).balance == 1, "deployment value");
        require(address(empty).code.length > 0, "no-args deployment");
    }
}
