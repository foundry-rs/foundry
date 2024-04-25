// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

// https://github.com/foundry-rs/foundry/issues/5868
contract Owned {
    address public owner;
    address private ownerCandidate;

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    modifier onlyOwnerCandidate() {
        require(msg.sender == ownerCandidate);
        _;
    }

    function transferOwnership(address candidate) external onlyOwner {
        ownerCandidate = candidate;
    }

    function acceptOwnership() external onlyOwnerCandidate {
        owner = ownerCandidate;
    }
}

contract Handler is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Owned owned;

    constructor(Owned _owned) {
        owned = _owned;
    }

    function transferOwnership(address sender, address candidate) external {
        vm.assume(sender != address(0));
        vm.prank(sender);
        owned.transferOwnership(candidate);
    }

    function acceptOwnership(address sender) external {
        vm.assume(sender != address(0));
        vm.prank(sender);
        owned.acceptOwnership();
    }
}

contract InvariantCalldataDictionary is DSTest {
    address owner;
    Owned owned;
    Handler handler;
    address[] actors;

    function setUp() public {
        owner = address(this);
        owned = new Owned();
        handler = new Handler(owned);
        actors.push(owner);
        actors.push(address(777));
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.transferOwnership.selector;
        selectors[1] = handler.acceptOwnership.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function fixtureSender() external returns (address[] memory) {
        return actors;
    }

    function fixtureCandidate() external returns (address[] memory) {
        return actors;
    }

    function invariant_owner_never_changes() public {
        assertEq(owned.owner(), owner);
    }
}
