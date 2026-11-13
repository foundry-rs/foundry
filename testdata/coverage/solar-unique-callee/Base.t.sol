// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;
import {Base, Harness} from "../src/Base.sol";
contract BaseTest {
    function testInit() public {
        Harness harness = new Harness();
        harness.init(address(1));
        assert(harness.a() == address(1));
    }
    function testReject() public {
        Harness harness = new Harness();
        (bool ok, bytes memory data) = address(harness).call(
            abi.encodeCall(harness.init, (address(0)))
        );
        assert(!ok);
        assert(data.length == 4);
        assert(bytes4(data) == Base.ZeroAddress.selector);
    }
}
