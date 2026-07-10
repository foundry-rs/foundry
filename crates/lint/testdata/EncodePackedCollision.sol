//@compile-flags: --only-lint encode-packed-collision

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract EncodePackedCollision {
    // SHOULD WARN: two string args
    function twoStrings(string memory a, string memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: two bytes args
    function twoBytes(bytes memory a, bytes memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: string + bytes
    function stringAndBytes(string memory a, bytes memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: dynamic array + string
    function arrayAndString(uint256[] memory a, string memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: two dynamic arrays
    function twoArrays(uint256[] memory a, address[] memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: three dynamic args, still one call
    function threeStrings(string memory a, string memory b, string memory c) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b, c)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD PASS: only one dynamic arg
    function oneString(string memory a, uint256 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: no dynamic args
    function noDynamic(uint256 a, address b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: fixed bytes are not dynamic
    function fixedBytes(bytes32 a, bytes32 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: fixed-size array is not dynamic
    function fixedArray(uint256[3] memory a, uint256[3] memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, b));
    }

    // SHOULD PASS: single string arg
    function singleString(string memory a) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(a));
    }

    // SHOULD WARN: call returning string/bytes
    function getString() internal pure returns (string memory) { return "x"; }
    function getBytes() internal pure returns (bytes memory) { return hex"ff"; }

    function callReturns() public pure returns (bytes32) {
        return keccak256(abi.encodePacked(getString(), getBytes())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: explicit type casts to bytes/string
    function typeCasts(bytes32 a, bytes32 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(bytes(abi.encode(a)), bytes(abi.encode(b)))); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD PASS: two string literals are compile-time constants; no runtime variability
    function stringLiterals() public pure returns (bytes32) {
        return keccak256(abi.encodePacked("a", "bc"));
    }

    // SHOULD PASS: single string literal
    function singleLiteral() public pure returns (bytes32) {
        return keccak256(abi.encodePacked("abc"));
    }

    // SHOULD PASS: one literal + one dynamic is still injective
    function literalPrefixSafe(string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked("prefix:", s));
    }

    // SHOULD PASS: hex literal + one dynamic is still injective
    function hexLiteralSafe(bytes memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(hex"dead", b));
    }
}

// SHOULD WARN: concrete contract overriding inherited name()/symbol(), both return string,
// so even though contract_item_ids yields two entries per method the lint must still fire.
interface IToken {
    function name() external view returns (string memory);
    function symbol() external view returns (string memory);
}

contract ConcreteToken is IToken {
    function name() external pure override returns (string memory) { return "Token"; }
    function symbol() external pure override returns (string memory) { return "TKN"; }
}

contract OverrideCollision {
    function tokenCollision(ConcreteToken token) public view returns (bytes32) {
        return keccak256(abi.encodePacked(token.name(), token.symbol())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD NOT WARN: overloaded function, must pick the correct overload based on arg type
contract OverloadFP {
    function f(uint256) internal pure returns (string memory) { return ""; }
    function f(bytes32) internal pure returns (uint256) { return 1; }

    // f(x) resolves to f(bytes32) → returns uint256 (not dynamic); only s is dynamic → no warn
    function g(bytes32 x, string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(f(x), s));
    }
}

interface IERC20Metadata {
    function name() external view returns (string memory);
    function symbol() external view returns (string memory);
}

contract MemberCalls {
    // SHOULD WARN: external member calls returning string
    function tokenCollision(IERC20Metadata token) public view returns (bytes32) {
        return keccak256(abi.encodePacked(token.name(), token.symbol())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD PASS: single external member call
    function singleMemberCall(IERC20Metadata token) public view returns (bytes32) {
        return keccak256(abi.encodePacked(token.name()));
    }
}

// Explicit interface casts; IERC20Metadata(addr).name()
contract InterfaceCast {
    // SHOULD WARN: interface cast receiver
    function castCollision(address addr) public view returns (bytes32) {
        return keccak256(abi.encodePacked(IERC20Metadata(addr).name(), IERC20Metadata(addr).symbol())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// Contract-typed struct fields and array elements
contract TokenRegistry {
    struct Config { IERC20Metadata token; }

    // SHOULD WARN: struct field receiver
    function structField(Config memory cfg) public view returns (bytes32) {
        return keccak256(abi.encodePacked(cfg.token.name(), cfg.token.symbol())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: array index receiver
    function arrayIndex(IERC20Metadata[] memory tokens) public view returns (bytes32) {
        return keccak256(abi.encodePacked(tokens[0].name(), tokens[0].symbol())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// Member overloads with different arities; f() is dynamic, f(uint256) is not
contract OverloadArity {
    function f() internal pure returns (string memory) { return ""; }
    function f(uint256) internal pure returns (uint256) { return 1; }

    // SHOULD WARN: f() (0 args) unambiguously returns string; s is also dynamic
    function g(string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(f(), s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// Ternary expressions
contract TernaryDynamic {
    // SHOULD WARN: ternary where both branches are dynamic strings
    function ternaryCollision(bool flag, string memory a, string memory b, string memory c) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(flag ? a : b, c)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// Calldata slices
contract SliceDynamic {
    // SHOULD WARN: calldata slice (still bytes) + another dynamic arg
    function sliceCollision(bytes calldata data, string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(data[:4], s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// Fixed-size arrays with dynamic element types (e.g. string[2], bytes[2]).
// Note: solc itself rejects these in abi.encodePacked with "Type not supported in packed mode",
// so no lint fixture is needed, the compiler prevents the problematic pattern.

// SHOULD WARN: ternary where one branch is a string literal (literal has no expr_type)
contract TernaryLiteral {
    function ternaryLiteralCollision(bool flag, string memory a, string memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(flag ? a : "x", b)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: abi.encode() returns bytes, no explicit bytes() cast needed
contract AbiEncodeNoCast {
    function abiEncodeCollision(bytes32 a, bytes32 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(abi.encode(a), abi.encode(b))); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    // SHOULD WARN: nested abi.encodePacked returns bytes
    function nestedEncodePackedCollision(string memory a, string memory b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(abi.encodePacked(a), abi.encodePacked(b))); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: string.concat() returns string
contract StringConcatArg {
    function stringConcatCollision(string memory a, string memory b, string memory c) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(string.concat(a, b), c)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: bytes.concat() returns bytes
contract BytesConcatArg {
    function bytesConcatCollision(bytes memory a, bytes memory b, bytes memory c) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(bytes.concat(a, b), c)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: msg.data (bytes calldata) + string
contract MsgDataCollision {
    function msgDataAndString(string memory s) external pure returns (bytes32) {
        return keccak256(abi.encodePacked(msg.data, s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: address.code (bytes memory) + string
contract AddrCodeCollision {
    function addrCodeAndString(address addr, string memory s) public view returns (bytes32) {
        return keccak256(abi.encodePacked(addr.code, s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }

    function thisCodeAndString(string memory s) public view returns (bytes32) {
        return keccak256(abi.encodePacked(address(this).code, s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD PASS: a user-defined struct field named `code` is not address.code bytes.
contract StructCodeField {
    struct S { uint256 code; }

    function structCodeAndString(S memory x, string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(x.code, s));
    }
}

// SHOULD WARN: new bytes() + string
contract NewBytesCollision {
    function newBytesAndString(uint256 n, string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(new bytes(n), s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: public string state variable; synthesized getter returns string
contract PublicGetterToken {
    string public name = "Token";
    string public symbol = "TKN";
}
contract PublicGetterCollision {
    function getterCollision(PublicGetterToken t) public view returns (bytes32) {
        return keccak256(abi.encodePacked(t.name(), t.symbol())); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: library function returning string
library StringLib {
    function toStr(uint256) internal pure returns (string memory) { return ""; }
}
contract LibraryCallCollision {
    function libraryStringCollision(uint256 a, uint256 b) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(StringLib.toStr(a), StringLib.toStr(b))); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}

// SHOULD WARN: overload where the selected overload returns string.
contract OverloadShouldWarnFN {
    function f(uint256) internal pure returns (string memory) { return ""; }
    function f(bytes32) internal pure returns (uint256) { return 0; }

    // f(x) resolves to f(uint256) -> string (dynamic)
    function g(uint256 x, string memory s) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(f(x), s)); //~WARN: `abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible
    }
}
