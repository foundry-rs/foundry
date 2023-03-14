// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";
import "../logs/console.sol";

// https://github.com/foundry-rs/foundry/issues/4556
contract Issue4556Test is DSTest {
    showMsgSender public show;
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    address public phoebe = vm.addr(15);

    event LogAddress(address);

    function setUp() public {
        show = new showMsgSender();
        vm.label(phoebe, "Phoebe");
    }

    function testMSGSender() public {
        vm.startPrank(phoebe);
        within();
//        within();
        vm.stopPrank();
    }

    function within() public {
        emit LogAddress(msg.sender);  // THE BUG HAPPENS HERE
//        show.show();
    }
}

contract showMsgSender {
    event LogAddress(address);

    function show() public {
        emit LogAddress(msg.sender);
    }
}
