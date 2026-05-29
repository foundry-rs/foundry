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

// ── Inheritance: concrete base analyzed independently from derived ────────────
// Each concrete contract is checked on its own; Base can be deployed directly
// and `data` is never written in its scope, so it is flagged. DerivedGood
// writes `data` in its constructor, so no warning fires for DerivedGood.

contract Base {
    uint256 public data; //~WARN: state variable is read but never written

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

// ── Abstract contracts: not directly deployed, skip entirely ──────────────────

abstract contract AbstractBase {
    uint256 public unset; // would fire if abstract were not skipped

    function getUnset() public view returns (uint256) {
        return unset;
    }
}

// ── Mappings: always default-initialized, skip to avoid false positives ───────

contract ReadOnlyMapping {
    mapping(address => uint256) public balances; // public getter reads it; no explicit write

    function get(address a) public view returns (uint256) {
        return balances[a];
    }
}

// ── Storage-ref internal call: _set(data, v) writes `data` via storage ref ───

contract StorageRefCall {
    struct Data { uint256 val; }
    Data public slot;

    function _set(Data storage target, uint256 v) internal {
        target.val = v;
    }

    function set(uint256 v) external {
        _set(slot, v);
    }

    function get() public view returns (uint256) {
        return slot.val;
    }
}

// ── Library dispatch: data.set(v) writes `data` via `using Lib for Data` ────

library DataLib {
    struct Data { uint256 val; }
    function set(Data storage self, uint256 v) internal {
        self.val = v;
    }
    function get(Data storage self) internal view returns (uint256) {
        return self.val;
    }
}

contract LibraryDispatch {
    using DataLib for DataLib.Data;
    DataLib.Data public slot;

    function set(uint256 v) external {
        slot.set(v);
    }

    function get() public view returns (uint256) {
        return slot.get();
    }
}

// ── Concrete base with a concrete derived that writes the variable ────────────
// Deploying the base directly leaves `x` uninitialized; the derived constructor
// writing `x` only matters for that derived deployment.

contract BaseDeployable {
    uint256 public x; //~WARN: state variable is read but never written
    function get() external view returns (uint256) { return x; }
}

contract DerivedFromBaseDeployable is BaseDeployable {
    constructor() { x = 42; }
}

// ── Overloaded internal function ──────────────────────────────────────────────
// When overloads exist and any of them takes `storage` at that position, the
// argument is conservatively treated as written to avoid false positives.
// The write is only suppressed when NO overload has a storage parameter.

contract OverloadUnion {
    struct S { uint256 v; }
    uint256 public x; // any-overload: f(S storage) has storage at pos 0 → x treated as written
    S internal data;

    function f(uint256) internal {}
    function f(S storage) internal {}

    function callIt() external { f(x); }
    function read() external view returns (uint256) { return x; }
}

// ── Overloaded internal function: storage overload is the correct match ───────
// `set(slot)` where `slot` is `Data storage`; only `set(Data storage)` can
// accept a struct argument. Previously flagged as uninitialized with `all_storage`.

contract OverloadStorageWrite {
    struct Data { uint256 val; }
    Data public slot;

    function set(Data storage target, uint256 v) internal { target.val = v; }
    function set(uint256) internal {}

    function write(uint256 v) external { set(slot, v); }
    function get() public view returns (uint256) { return slot.val; }
}

// ── Storage-ref internal call with named arguments ────────────────────────────
// `_set({v: v, target: slot})` passes `slot` as the storage parameter even
// though it appears second in source order; the lint must match by name.

contract NamedArgStorage {
    struct Data { uint256 val; }
    Data public slot;

    function _set(Data storage target, uint256 v) internal { target.val = v; }
    function set(uint256 v) external { _set({v: v, target: slot}); }
    function get() public view returns (uint256) { return slot.val; }
}

// ── Named args, out-of-order: positional loop must not misfire ───────────────
// `_set({target: slot, v: x})` passes `x` as `v` (not storage) and `slot` as
// `target` (storage).  Before the fix the positional enumerate loop would see
// arg[1]=x against parameter[1]=target (storage) and falsely mark x as written.

contract NamedArgMisfire {
    struct Data { uint256 val; }
    Data slot;
    uint256 public x; //~WARN: state variable is read but never written

    function _set(uint256 v, Data storage target) internal { target.val = v; }
    function callIt() external { _set({target: slot, v: x}); }
    function read() external view returns (uint256) { return x; }
}

// ── Initializer side effect: _init() writes `y` while initializing `x` ───────

contract InitSideEffect {
    uint256 public y; // written via _init(), must NOT warn
    uint256 public x = _init();

    function _init() internal returns (uint256) {
        y = 1;
        return 2;
    }

    function getY() external view returns (uint256) { return y; }
}

// ── payable() cast on a member-call receiver is not a write ──────────────────
// `payable(owner).transfer(...)` reads `owner` to obtain an address; it does
// not write to the state variable.

contract PayableTransfer {
    address public owner; //~WARN: state variable is read but never written

    function withdraw() external {
        payable(owner).transfer(address(this).balance);
    }

    function getOwner() public view returns (address) { return owner; }
}

// ── Qualified inherited helper: Contract.f(slot) writes via storage ref ───────
// `BaseSetter._set(slot, v)` is a member callee; the lint must look up `_set`
// in `BaseSetter` and detect the `storage` parameter to avoid a false positive.

contract BaseSetter {
    struct Data { uint256 val; }
    function _set(Data storage target, uint256 v) internal { target.val = v; }
}

contract QualifiedInheritedWrite is BaseSetter {
    Data public slot;

    function write(uint256 v) external { BaseSetter._set(slot, v); }
    function get() public view returns (uint256) { return slot.val; }
}

// ── super.f(slot) writes via storage ref in parent ───────────────────────────
// `super._set(slot, v)` uses the `super` builtin as a member base; the lint
// must search the linearized inheritance chain for `_set` and detect the
// `storage` parameter.

contract SuperWrite is BaseSetter {
    Data public slot;

    function write(uint256 v) external { super._set(slot, v); }
    function get() public view returns (uint256) { return slot.val; }
}

// ── super.f(x): child storage overload must NOT suppress warning ──────────────
// The child defines `_init(Data storage)` but `super._init(slot)` resolves to
// the parent's `_init(Data memory)` (no storage parameter), so `slot` is never
// actually written through the super call and the warning must still fire.

contract SuperNonStorageBase {
    struct Data { uint256 val; }
    function _init(Data memory) internal virtual {}
}

contract SuperNonStorageChild is SuperNonStorageBase {
    Data public slot; //~WARN: state variable is read but never written

    // Child overload with storage param – must NOT affect super dispatch lookup
    function _init(Data storage target) internal { target.val = 1; }

    function use_() external view returns (uint256) { return slot.val; }
    function init() external { super._init(slot); }
}
