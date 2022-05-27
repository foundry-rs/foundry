// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";



struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract UnbreakableMockToken {
    uint256 public totalSupply = 0;

    function shouldRevert() public {
        require(false);
    }

    function mint(uint256 _a) public {
        totalSupply *= _a;
    }

    function harmless() public {}
}

contract BreakableMockToken {
    uint256 public totalSupply = 0;

    function shouldRevert() public {
        require(false);
        totalSupply += 5;
    }

    function mint(uint256 _a) public {
        totalSupply += 5;
    }
}

contract Jesus {
    address fren;
    bool public identity_revealed;

    function create_fren() public {
        fren = address(new Judas());
    }

    function kiss() public {
        require(msg.sender == fren);
        identity_revealed = true;
    }
}

contract Judas {
    Jesus jesus;

    constructor() {
        jesus = Jesus(msg.sender);
    }

    function betray() public {
        jesus.kiss();
    }
}

contract InvariantInnerContract is DSTest {
    Jesus jesus;

    function setUp() public {
        jesus = new Jesus();
    }

    function invariantHideJesus() public {
        emit log("invariantTestFailLog");
        require(jesus.identity_revealed() == false, "jesus betrayed.");
    }
}

contract InvariantFuzzTest is DSTest {
    UnbreakableMockToken ignore;
    UnbreakableMockToken unbreakable;
    BreakableMockToken breakable;

    function setUp() public {
        unbreakable = new UnbreakableMockToken();
        ignore = new UnbreakableMockToken();
        breakable = new BreakableMockToken();
    }

    function excludeContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](2);
        addrs[0] = address(unbreakable);
        addrs[1] = address(ignore);
        return addrs;
    }

    function targetContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](2);
        addrs[0] = address(breakable);
        addrs[1] = address(unbreakable);
        return addrs;
    }

    function targetSenders() public returns (address[] memory) {
        address[] memory addrs = new address[](2);
        addrs[0] = address(0x1337);
        addrs[1] = address(0x1338);
        return addrs;
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = BreakableMockToken.shouldRevert.selector;
        targets[0] = FuzzSelector(address(breakable), selectors);
        return targets;
    }

    function invariantTestPass2() public {
        emit log("invariantTestPassLog");
        require(unbreakable.totalSupply() < 10, "should not revert");
    }

    function invariantTestPass() public {
        emit log("invariantTestPassLog");
        require(unbreakable.totalSupply() < 10, "should not revert");
    }

    function invariantTestFail() public {
        emit log("invariantTestFailLog");
        require(breakable.totalSupply() < 10, "should revert");
    }
}


contract C1 {
    address fren;

    function create_fren() public returns (address) {
        require(fren == address(0), "");
        fren = address(new C2());
        return fren;
    }

    function kiss() public {
    }
}

contract C2 {
    C1 jesus;
    bool public betraying = false;
    bool public payback = false;

    constructor() {
        jesus = C1(msg.sender);
    }

    function betray() public {
        betraying = true;
        jesus.kiss();
        betraying = false;
    }
    event e(string);
    function revenge() public returns (string memory) {
        require(betraying, "");
        emit e("rere)");
        payback = true;
        return "oi";
    }
}

contract InvariantReentrancy is DSTest {
    C1 jesus;
    C2 fren;


    function excludeContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](2);
        addrs[0] = address(jesus);
        addrs[1] = address(fren);
        return addrs;
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[1] = C2.betray.selector;
        selectors[0] = C2.revenge.selector;
        targets[0] = FuzzSelector(address(fren), selectors);
        return targets;
    }


    function setUp() public {
        jesus = new C1();
        fren = C2(jesus.create_fren());
    }

    function invariantPayback() public {
        emit log("invariantTestFailLog");
        require(fren.payback() == false, "payback.");
    }
}
