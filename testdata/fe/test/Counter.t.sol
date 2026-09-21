// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

interface Vm {
    function getCode(string calldata artifactPath) external view returns (bytes memory);
    function expectEmit(bool topic1, bool topic2, bool topic3, bool data, address emitter) external;
    function expectRevert(bytes calldata revertData) external;
}

interface ICounter {
    function number() external view returns (uint256);
    function setNumber(uint256 value) external;
    function increment() external;
}

contract CounterTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ICounter counter;
    event NumberChanged(uint256 value);

    function setUp() public {
        // Forge resolves the Fe contract through its standard artifact API.
        bytes memory code = abi.encodePacked(vm.getCode("src/counter/src/lib.fe:Counter"), abi.encode(uint256(41)));
        address deployed;
        assembly { deployed := create(0, add(code, 0x20), mload(code)) }
        require(deployed != address(0), "Fe deployment failed");
        counter = ICounter(deployed);
    }

    function testConstructor() public view {
        require(counter.number() == 41, "constructor value");
    }

    function testIncrementUsesFeDependency() public {
        counter.increment();
        require(counter.number() == 42, "increment through local Fe dependency");
    }

    function testEmitsEvent() public {
        vm.expectEmit(false, false, false, true, address(counter));
        emit NumberChanged(123);
        counter.setNumber(123);
    }

    function testOverflowReverts() public {
        counter.setNumber(type(uint256).max);
        vm.expectRevert(abi.encodeWithSignature("Panic(uint256)", uint256(0x11)));
        counter.increment();
    }

    function testFuzzSetNumber(uint256 value) public {
        counter.setNumber(value);
        require(counter.number() == value, "round trip");
    }
}
