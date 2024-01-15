// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract SignTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSignP256() public {
        bytes32 pk = hex"0A1D0F90D7299AF6F990FD67A2D535D2FD82BCDA9B9AA352E08B262C7021573C";
        bytes32 digest= hex"54705ba3baafdbdfba8c5f9a70f7a89bee98d906b53e31074da7baecdc0da9ad";

        (bytes32 r, bytes32 s) = vm.signP256(uint256(pk), digest);
        assertEq(r, hex"995F5AC4CA7EFCA7DF7BA0849D5BBC4C8B9621F909ED8E6E0CD3472D252FF5FF");
        assertEq(s, hex"3E98D081DE4177BCDF01A8DB1423DB32152B437C56B66C3A52A27FB4C48689A8");
    }
}
