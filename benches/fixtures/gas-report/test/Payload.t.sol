// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract Payload {
    function roundTrip(uint256[] calldata) external pure returns (uint256[] memory) {
        assembly {
            let n := sub(calldatasize(), 4)
            calldatacopy(0, 4, n)
            return(0, n)
        }
    }
}

contract PayloadTest {
    Payload payload;

    function setUp() public {
        payload = new Payload();
    }

    function testPayload() public view {
        address target = address(payload);
        uint256 selector = uint32(Payload.roundTrip.selector);
        assembly {
            let p := mload(0x40)
            mstore(p, shl(224, selector))
            mstore(add(p, 4), 32)
            mstore(add(p, 36), 8192)
            let end := add(p, add(68, mul(8192, 32)))
            for { let q := add(p, 68) } lt(q, end) { q := add(q, 32) } {
                mstore(q, not(0))
            }
            for { let i := 0 } lt(i, 128) { i := add(i, 1) } {
                if iszero(staticcall(gas(), target, p, sub(end, p), 0, 0)) { revert(0, 0) }
                if iszero(eq(returndatasize(), sub(sub(end, p), 4))) { revert(0, 0) }
            }
        }
    }
}
