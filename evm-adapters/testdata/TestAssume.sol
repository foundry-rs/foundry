pragma solidity 0.8.0;

import "../../evm-adapters/testdata/DsTest.sol";

interface HEVM {
    function assume(bool condition) external;
}

address constant HEVM_ADDRESS =
    address(bytes20(uint160(uint256(keccak256('hevm cheat code')))));


contract TestAssume is DSTest {
    HEVM constant hevm = HEVM(HEVM_ADDRESS);
    function testAssume(uint8 x) public {
        hevm.assume(x < 2**7);
        assertTrue(x < 2**7);
    }
}
