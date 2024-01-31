// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

// https://github.com/foundry-rs/foundry/issues/6966
// See also https://github.com/RustCrypto/elliptic-curves/issues/988#issuecomment-1817681013
contract Issue6966Test is DSTest {
    function testEcrecover() public {
        bytes32 h = 0x0000000000000000000000000000000000000000000000000000000000000000;
        uint8 v = 27;
        bytes32 r = bytes32(0xf87fff3202dfeae34ce9cb8151ce2e176bee02a937baac6de85c4ea03d6a6618);
        bytes32 s = bytes32(0xedf9ab5c7d3ec1df1c2b48600ab0a35f586e069e9a69c6cdeebc99920128d1a5);
        assert(ecrecover(h, v, r, s) != address(0));
    }
}
