// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract Payable {
    function pay() payable public {}
}

contract PaymentFailureTest is DSTest {
}
