//@compile-flags: --only-lint redundant-base-constructor-call
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract NoCtor {}

contract EmptyCtor {
    constructor() {}
}

contract NonEmptyZeroParamCtor {
    uint256 public x;
    constructor() {
        x = 1;
    }
}

contract WithArgs {
    uint256 public y;
    constructor(uint256 _y) {
        y = _y;
    }
}

// SHOULD FAIL:

contract A1 is NoCtor() {} //~NOTE: explicit empty base-constructor arguments are redundant

contract A2 is EmptyCtor() {} //~NOTE: explicit empty base-constructor arguments are redundant

contract A3 is NonEmptyZeroParamCtor() {} //~NOTE: explicit empty base-constructor arguments are redundant

contract A4 is NoCtor {
    constructor() NoCtor() {} //~NOTE: explicit empty base-constructor arguments are redundant
}

contract A5 is EmptyCtor {
    constructor() EmptyCtor() {} //~NOTE: explicit empty base-constructor arguments are redundant
}

contract A6 is NoCtor(), EmptyCtor() {}
//~^ NOTE: explicit empty base-constructor arguments are redundant
//~| NOTE: explicit empty base-constructor arguments are redundant

contract A7 is NoCtor, WithArgs {
    modifier m() { _; }
    constructor(uint256 v) m() WithArgs(v) NoCtor() {}
    //~^ NOTE: explicit empty base-constructor arguments are redundant
}

// SHOULD PASS:

contract Ok1 is NoCtor {}

contract Ok2 is EmptyCtor {}

contract Ok3 is WithArgs(1) {}

contract Ok4 is WithArgs {
    constructor(uint256 v) WithArgs(v) {}
}

contract Mid is NoCtor {}
contract Ok5 is Mid {}
