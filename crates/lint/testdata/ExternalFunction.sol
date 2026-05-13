//@compile-flags: --only-lint external-function

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IExternal {
    function ping(bytes calldata data) external;
}

// Library functions are out of scope (different `external` semantics).
library MathLib {
    function sumMemory(uint256[] memory xs) public pure returns (uint256 s) {
        for (uint256 i = 0; i < xs.length; i++) s += xs[i];
    }
}

contract ExternalFunction {
    address public owner;
    bytes public stored;

    // SHOULD FAIL:

    function setStored(bytes memory data) public { //~NOTE: public function can be declared external
        stored = data;
    }

    function multiArrayConsumer(uint256[] memory xs, address[] memory ys) public { //~NOTE: public function can be declared external
        owner = ys[0];
        if (xs.length > 0) owner = ys[xs.length - 1];
    }

    function structConsumer(Item memory item) public { //~NOTE: public function can be declared external
        owner = item.who;
    }

    function nestedReferenceConsumer(string memory s, uint256 v) public returns (uint256) { //~NOTE: public function can be declared external
        bytes memory bs = bytes(s);
        return bs.length + v;
    }

    function calledOnlyExternally(bytes memory payload) public { //~NOTE: public function can be declared external
        stored = payload;
    }

    // SHOULD PASS:

    // Already external.
    function alreadyExternal(bytes calldata data) external {
        stored = data;
    }

    // Internal / private — not candidates.
    function internalHelper(bytes memory data) internal {
        stored = data;
    }

    function privateHelper(bytes memory data) private {
        stored = data;
    }

    // Value-only signature — savings are negligible.
    function valueOnly(uint256 a, uint256 b) public {
        owner = address(uint160(a + b));
    }

    // Reference param already in calldata.
    function calldataReferenceOnly(bytes calldata data) public {
        stored = data;
    }

    // Called internally — must stay public.
    function calledInternally(bytes memory data) public {
        stored = data;
    }

    function callsCalledInternally(bytes memory data) external {
        calledInternally(data);
    }

    // Used as a function pointer — counted as an internal use.
    function takenAsPointer(bytes memory data) public {
        stored = data;
    }

    function usePointer(bytes memory data) external {
        function (bytes memory) internal ptr = takenAsPointer;
        ptr(data);
    }

    // Body writes to a parameter — switching to calldata would not compile.
    function writesToParam(bytes memory data) public {
        data = abi.encodePacked(data, uint8(0x01));
        stored = data;
    }

    function writesToParamField(Item memory item) public {
        item.who = msg.sender;
        owner = item.who;
    }

    function writesToParamIndex(uint256[] memory xs) public {
        xs[0] = 1;
        owner = address(uint160(xs[0]));
    }

    function deletesParam(uint256[] memory xs) public {
        delete xs;
    }

    function incrementsParam(uint256 i, uint256[] memory xs) public {
        xs[i++] = 1;
    }

    // Constructor / receive / fallback — never candidates.
    constructor(bytes memory init) {
        stored = init;
    }

    receive() external payable {}

    fallback() external payable {
        stored = msg.data;
    }

    struct Item {
        address who;
        uint256 amount;
    }
}

abstract contract Base {
    bytes internal _bytes;

    // Abstract — must stay ≥ public for derived contracts to override.
    function virtualWithoutBody(bytes memory data) public virtual;

    // Reached via `super.calledViaSuper(...)` in `Derived`; matched by name.
    function calledViaSuper(bytes memory data) public virtual {
        _bytes = data;
    }
}

contract Derived is Base {
    // Override of a public base — skipped regardless of internal use.
    function virtualWithoutBody(bytes memory data) public override {
        _bytes = data;
    }

    function callsSuper(bytes memory data) external {
        super.calledViaSuper(data);
    }
}

// Interface functions have no body and are skipped.
interface IIface {
    function ifaceFn(bytes calldata data) external;
}
