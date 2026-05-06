//@compile-flags: --only-lint could-be-immutable

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract CouldBeImmutable {
    uint256 public constant MAX = 10;
    uint256 public immutable ALREADY_IMMUTABLE;

    address public owner;
    address public deployer = msg.sender;
    uint256 private configured;
    bytes32 internal salt = keccak256(abi.encodePacked(block.timestamp));
    CouldBeImmutable private peer;

    uint256 private mutableValue;
    uint256 private assignedInInternal;
    uint256 private compileTimeConstant = 1 + 2;
    string private dynamicValue;

    constructor(uint256 configured_, CouldBeImmutable peer_, string memory value) {
        ALREADY_IMMUTABLE = configured_;
        owner = msg.sender;
        configured = configured_;
        peer = peer_;
        mutableValue = 1;
        setInternal(1);
        dynamicValue = value;
    }

    function setMutableValue(uint256 newValue) public {
        mutableValue = newValue;
    }

    function setInternal(uint256 newValue) internal {
        assignedInInternal = newValue;
    }
}

contract BaseImmutableCandidate {
    uint256 internal inheritedBase;
}

contract DerivedImmutableCandidate is BaseImmutableCandidate {
    constructor(uint256 value) {
        inheritedBase = value;
    }
}

contract BaseConstructorImmutableCandidate {
    uint256 internal baseConfigured;

    constructor(uint256 value) {
        baseConfigured = value;
    }
}

contract DerivedConstructorImmutableCandidate is BaseConstructorImmutableCandidate {
    constructor(uint256 value) BaseConstructorImmutableCandidate(value) {}
}

contract ModifierBodyWrite {
    uint256 private fromModifier;

    modifier initializesState() {
        fromModifier = 1;
        _;
    }

    constructor() initializesState() {}
}

contract AssemblyWrite {
    uint256 private fromAssembly;

    constructor() {
        fromAssembly = 1;
    }

    function mutate() public {
        assembly {
            sstore(0, 2)
        }
    }
}
