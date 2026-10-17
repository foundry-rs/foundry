// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.15 <0.9.0;

import {DeploymentState, Empty, Target, Reverting} from "../src/Targets.sol";

interface Vm {
    function getNonce(address account) external view returns (uint64);
    function computeCreateAddress(address deployer, uint256 nonce) external pure returns (address);
    function computeCreate2Address(bytes32 salt, bytes32 initCodeHash, address deployer) external pure returns (address);
    function deal(address account, uint256 balance) external;
    function prank(address sender, address origin) external;
    function startPrank(address sender, address origin) external;
    function stopPrank() external;
    function broadcast(address sender) external;
    function startBroadcast(address sender) external;
    function stopBroadcast() external;
}

abstract contract DeploymentAssertions {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 internal constant SALT = bytes32(uint256(7));

    function check(DeploymentState deployed, address expected, address caller, address origin, uint256 value)
        internal
        view
    {
        require(address(deployed) == expected, "deployment address");
        require(deployed.deployedAt() == expected, "constructor address");
        require(deployed.caller() == caller, "constructor caller");
        require(deployed.origin() == origin, "constructor origin");
        require(deployed.value() == value && expected.balance == value, "constructor value");
        require(expected.code.length > 0, "deployed code");
    }

    function nextAddress(address caller) internal view returns (address) {
        return vm.computeCreateAddress(caller, vm.getNonce(caller));
    }

    function saltedAddress(bytes memory code) internal view returns (address) {
        return vm.computeCreate2Address(SALT, keccak256(code), address(this));
    }

    function checkArgs(Target target) internal view {
        require(target.first() == 11 && target.second() == 22, "constructor arguments");
    }
}

contract DeploymentTest is DeploymentAssertions {
    uint256 internal argumentCalls;

    function test_create() public {
        address expected = nextAddress(address(this));
        check(new Empty(), expected, address(this), tx.origin, 0);
    }

    function test_create_args() public {
        address expected = nextAddress(address(this));
        Target target = new Target(11, 22);
        check(target, expected, address(this), tx.origin, 0);
        checkArgs(target);
    }

    function test_create_value() public {
        address expected = nextAddress(address(this));
        check(new Empty{value: 3}(), expected, address(this), tx.origin, 3);
    }

    function test_create_args_value() public {
        address expected = nextAddress(address(this));
        Target target = new Target{value: 5}(11, 22);
        check(target, expected, address(this), tx.origin, 5);
        checkArgs(target);
    }

    function test_create2() public {
        address expected = saltedAddress(type(Empty).creationCode);
        check(new Empty{salt: SALT}(), expected, address(this), tx.origin, 0);
    }

    function test_create2_args() public {
        address expected =
            saltedAddress(abi.encodePacked(type(Target).creationCode, abi.encode(uint256(11), uint256(22))));
        Target target = new Target{salt: SALT}(11, 22);
        check(target, expected, address(this), tx.origin, 0);
        checkArgs(target);
    }

    function test_create2_value() public {
        address expected = saltedAddress(type(Empty).creationCode);
        check(new Empty{value: 3, salt: SALT}(), expected, address(this), tx.origin, 3);
    }

    function test_create2_args_value() public {
        address expected =
            saltedAddress(abi.encodePacked(type(Target).creationCode, abi.encode(uint256(11), uint256(22))));
        Target target = new Target{salt: SALT, value: 5}(11, 22);
        check(target, expected, address(this), tx.origin, 5);
        checkArgs(target);
    }

    function test_named_side_effect_arguments() public {
        address expected = nextAddress(address(this));
        Target target = new Target({second_: argument(22), first_: argument(11)});
        check(target, expected, address(this), tx.origin, 0);
        checkArgs(target);
        require(argumentCalls == 2, "arguments evaluated more than once");
    }

    function argument(uint256 value) internal returns (uint256) {
        ++argumentCalls;
        return value;
    }

    function test_create2_side_effect_arguments() public {
        address expected =
            saltedAddress(abi.encodePacked(type(Target).creationCode, abi.encode(uint256(11), uint256(22))));
        Target target =
            new Target{value: argument(5), salt: bytes32(argument(7))}({second_: argument(22), first_: argument(11)});
        check(target, expected, address(this), tx.origin, 5);
        checkArgs(target);
        require(argumentCalls == 4, "arguments evaluated more than once");
    }

    function test_revert_data() public {
        (bool success, bytes memory result) = address(this).call(abi.encodeCall(this.deployReverting, ()));
        require(!success, "constructor succeeded");
        require(
            keccak256(result) == keccak256(abi.encodeWithSelector(Reverting.ConstructorReverted.selector, 17)),
            "revert data"
        );
    }

    function deployReverting() external {
        new Reverting(17);
    }

    function test_prank() public {
        address sender = address(0x1234);
        address origin = address(0x5678);
        address expected = nextAddress(sender);
        vm.prank(sender, origin);
        Empty target = new Empty();
        check(target, expected, sender, origin, 0);
        expected = nextAddress(address(this));
        check(new Empty(), expected, address(this), tx.origin, 0);
    }

    function test_recurrent_prank() public {
        address sender = address(0x1234);
        address origin = address(0x5678);
        address expected = nextAddress(sender);
        vm.startPrank(sender, origin);
        Empty first = new Empty();
        Empty second = new Empty();
        vm.stopPrank();
        check(first, expected, sender, origin, 0);
        check(second, vm.computeCreateAddress(sender, vm.getNonce(sender) - 1), sender, origin, 0);
    }

    function test_broadcast() public {
        address sender = address(0x1234);
        vm.deal(sender, 100);
        address expected = nextAddress(sender);
        vm.broadcast(sender);
        Empty target = new Empty{value: 5}();
        check(target, expected, sender, sender, 5);
        expected = nextAddress(address(this));
        check(new Empty(), expected, address(this), tx.origin, 0);
    }

    function test_recurrent_broadcast() public {
        address sender = address(0x1234);
        vm.deal(sender, 100);
        address expected = nextAddress(sender);
        vm.startBroadcast(sender);
        Empty first = new Empty{value: 5}();
        Empty second = new Empty{value: 7}();
        vm.stopBroadcast();
        check(first, expected, sender, sender, 5);
        check(second, vm.computeCreateAddress(sender, vm.getNonce(sender) - 1), sender, sender, 7);
    }
}

contract OtherOwnerTest is DeploymentAssertions {
    function test_other_owner() public {
        address expected = nextAddress(address(this));
        Target target = new Target({first_: 11, second_: 22});
        check(target, expected, address(this), tx.origin, 0);
        checkArgs(target);
    }
}
