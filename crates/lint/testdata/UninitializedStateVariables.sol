//@compile-flags: --only-lint uninitialized-state

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// ── Basic: read but never written ────────────────────────────────────────────

contract Bad {
    uint256 public balance; //~WARN: state variable is read but never written
    address public owner; //~WARN: state variable is read but never written

    function getBalance() public view returns (uint256) {
        return balance;
    }

    function isOwner(address addr) public view returns (bool) {
        return addr == owner;
    }
}

// ── Good: various write patterns ─────────────────────────────────────────────

contract GoodConstructor {
    address public owner;

    constructor() {
        owner = msg.sender;
    }

    function getOwner() public view returns (address) {
        return owner;
    }
}

contract GoodFunction {
    uint256 public counter;

    function inc() external {
        counter += 1;
    }

    function get() public view returns (uint256) {
        return counter;
    }
}

contract GoodInlineInit {
    uint256 public value = 42;

    function get() public view returns (uint256) {
        return value;
    }
}

// ── push / pop mutate a dynamic array ────────────────────────────────────────

contract GoodPushPop {
    uint256[] public items;

    function add(uint256 v) external {
        items.push(v);
    }

    function remove() external {
        items.pop();
    }

    function len() public view returns (uint256) {
        return items.length;
    }
}

// ── Inheritance: write in derived, read in base ───────────────────────────────

contract Base {
    uint256 public data;

    function getData() public view returns (uint256) {
        return data;
    }
}

contract DerivedGood is Base {
    constructor() {
        data = 1;
    }
}

// ── Inheritance: base var never written anywhere ──────────────────────────────

contract BaseUnwritten {
    uint256 public val; //~WARN: state variable is read but never written

    function getVal() public view returns (uint256) {
        return val;
    }
}

contract DerivedUnwritten is BaseUnwritten {}

// ── Modifier args: reading a state var as arg is a read, not a write ──────────

contract WithModifier {
    uint256 public cap; //~WARN: state variable is read but never written
    uint256 public used;

    modifier limited(uint256 max) {
        require(used <= max);
        _;
    }

    // cap is read as a modifier argument; used is written in the function body.
    function foo() external limited(cap) {
        used += 1;
    }
}

// ── Constants and immutables are always initialized ───────────────────────────

contract SkipConstAndImmutable {
    uint256 constant MAX = 100;
    uint256 immutable LIMIT;

    constructor(uint256 limit) {
        LIMIT = limit;
    }

    function check(uint256 x) public view returns (bool) {
        return x <= MAX && x <= LIMIT;
    }
}

// ── Never read: caught by unused-state-variables, not this lint ───────────────

contract SkipNeverRead {
    uint256 neverReadNorWritten;
}

// ── Assembly: bail conservatively, no false positives ────────────────────────

contract WithAssembly {
    uint256 public slot0; // would look unwritten, but assembly might write it

    function store(uint256 v) external {
        assembly {
            sstore(slot0.slot, v)
        }
    }

    function load() public view returns (uint256 result) {
        assembly {
            result := sload(slot0.slot)
        }
    }
}

// ── Mappings written via delete ───────────────────────────────────────────────

contract GoodMappingDelete {
    mapping(address => uint256) public balances;

    function clear(address user) external {
        delete balances[user];
    }

    function get(address user) public view returns (uint256) {
        return balances[user];
    }
}

// ── Synthetic getter counts as a read ────────────────────────────────────────
// A `public` state variable gets a compiler-synthesised getter. The HIR
// includes that getter as a function, so the variable is detected as read.

contract GetterOnly {
    uint256 public x; //~WARN: state variable is read but never written
}

contract GetterOnlyWritten {
    uint256 public x;

    constructor() {
        x = 1;
    }
}

// ── Base-constructor-arg: state var read in `constructor() A(x) {}` ───────────
// The modifier-arg `AcceptsArg(initVal)` on the derived constructor reads
// `initVal` from state; if it is never written the lint fires.

contract AcceptsArg {
    constructor(uint256) {}
}

contract BaseCtorArgRead is AcceptsArg {
    uint256 public initVal; //~WARN: state variable is read but never written
    constructor() AcceptsArg(initVal) {}
}

contract BaseCtorArgWritten is AcceptsArg {
    uint256 public initVal;
    constructor(uint256 v) AcceptsArg(initVal) {
        initVal = v; // written in body, so not flagged
    }
}
