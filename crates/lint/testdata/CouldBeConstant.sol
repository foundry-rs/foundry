//@compile-flags: --only-lint could-be-constant

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Other {
    uint256 public x;
}

interface IToken {
    function balanceOf(address) external view returns (uint256);
}

contract CouldBeConstant {
    // --- triggering cases (constant initializer, no writes anywhere) ---

    uint256 public limit = 100; //~NOTE: state variable could be declared constant
    uint256 internal sum = 1 + 2; //~NOTE: state variable could be declared constant
    bytes32 internal salt = keccak256("foundry"); //~NOTE: state variable could be declared constant
    string internal greeting = "hi"; //~NOTE: state variable could be declared constant
    bytes internal payloadPrefix = hex"0a0b"; //~NOTE: state variable could be declared constant
    uint256 internal derived = ALREADY_CONST + 1; //~NOTE: state variable could be declared constant
    IToken internal token = IToken(address(0xCAFE)); //~NOTE: state variable could be declared constant
    address internal nestedCast = address(uint160(0xCAFE)); //~NOTE: state variable could be declared constant
    uint256 internal maxUint = type(uint256).max; //~NOTE: state variable could be declared constant
    int256 internal minInt = type(int256).min; //~NOTE: state variable could be declared constant
    bytes4 internal iid = type(IToken).interfaceId; //~NOTE: state variable could be declared constant

    // --- non-triggering cases ---

    // Already declared constant / immutable.
    uint256 public constant ALREADY_CONST = 1;
    uint256 public immutable ALREADY_IMMUTABLE;

    // Non-constant initializer: should be flagged as `could-be-immutable`, not `could-be-constant`.
    address public deployer = msg.sender;

    // No initializer: not flagged (we require an inline value).
    uint256 internal noInit;

    // Written in non-constructor function: not flagged.
    uint256 internal mutated = 1;

    // Written in constructor body: would be `could-be-immutable`, not `could-be-constant`.
    uint256 internal configured = 1;

    // `new` calls are not compile-time constants.
    Other internal spawned = new Other();

    // Arrays / mappings are not constant-eligible types.
    uint256[] internal arr = [uint256(1), 2, 3];
    mapping(address => uint256) internal balances;

    // Written by a sibling state variable's initializer: not flagged.
    uint256 internal writtenByInitializer = 0;
    uint256 internal writerInitializer = (writtenByInitializer = 1);

    // Self-writing initializer: not flagged (initializer is non-constant).
    uint256 internal selfWriting = (selfWriting = 1);

    constructor(uint256 immutableValue) {
        ALREADY_IMMUTABLE = immutableValue;
        configured = immutableValue;
    }

    function bump() public {
        mutated += 1;
    }
}
