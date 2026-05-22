//@compile-flags: --only-lint assert-state-change

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract AssertStateChange {
    uint256 public counter;
    bool public flag;
    mapping(address => uint256) public balances;
    uint256[] public items;

    // Bad: pre-increment of state variable inside assert
    function badPreIncrement(uint256 expected) external {
        assert(++counter == expected); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: post-increment of state variable inside assert
    function badPostIncrement(uint256 expected) external {
        assert(counter++ == expected); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: call to state-mutating internal function
    function _toggleFlag() internal returns (bool) {
        flag = !flag;
        return flag;
    }

    function badMutatingCall() external {
        assert(_toggleFlag()); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: call to another state-mutating function
    function _deposit() internal returns (bool) {
        balances[msg.sender] += msg.value;
        return true;
    }

    function badDeposit() external payable {
        assert(_deposit()); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: state variable assignment inside assert
    function badAssignment(uint256 val) external {
        assert((counter = val) > 0); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: mapping index assignment (state variable lvalue)
    function badMappingAssign(address user, uint256 amt) external {
        assert((balances[user] = amt) > 0); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: pure comparison, no state change
    function goodComparison(uint256 expected) external view {
        assert(counter == expected);
    }

    // Good: view function call inside assert
    function _getCounter() internal view returns (uint256) {
        return counter;
    }

    function goodViewCall(uint256 expected) external view {
        assert(_getCounter() == expected);
    }

    // Good: require() with increment is fine (not assert)
    function goodRequire(uint256 expected) external {
        require(++counter == expected, "mismatch");
    }

    // Good: local variable increment, not a state variable
    function goodLocalInc() external pure returns (uint256) {
        uint256 local = 0;
        assert(++local == 1);
        return local;
    }

    // Good: local variable assignment, not a state variable
    function goodLocalAssign(uint256 val) external pure {
        uint256 local;
        assert((local = val) > 0);
    }
}

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
    // Overloaded: one mutating (2 args), one view (1 arg) — different arities so arity
    // narrowing can correctly distinguish them.
    function update(address target, uint256 amount) external returns (bool);
    function update(uint256 n) external view returns (bool);
}

// Interface with view functions that share names with low-level address builtins.
// These must NOT be flagged, the name-only heuristic must not fire when the receiver
// resolves to a known contract/interface.
interface IRouter {
    function send(uint256 amount) external view returns (bool);
    function call(bytes calldata data) external view returns (bool);
    function transfer(uint256 amount) external view returns (bool);
}

contract AssertStateChangeExternal {
    IToken public token;
    IRouter public router;
    address payable public recipient;

    // Bad: .send() always transfers ether (state-changing), returns bool
    function badSend() external {
        assert(recipient.send(1 ether)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: interface call to a non-view function
    function badInterfaceCall(address to, uint256 amt) external {
        assert(token.transfer(to, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view function on an interface does not mutate state
    function goodInterfaceView(uint256 expected) external view {
        assert(token.balanceOf(address(this)) >= expected);
    }

    // Good: calling the 1-arg view overload `update(uint256)` — arity differs from
    // the 2-arg mutating overload, so no false positive.
    function goodOverloadView(uint256 n) external view {
        assert(token.update(n));
    }

    // Bad: calling the 2-arg mutating overload `update(address,uint256)` — only
    // overload with this arity, so correctly flagged.
    function badOverloadMutating(address target, uint256 amt) external {
        assert(token.update(target, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view functions on an interface that happen to be named send/call/transfer
    // must NOT trigger the name-only address heuristic (fix for false positives).
    function goodViewSend(uint256 n) external view {
        assert(router.send(n));
    }

    function goodViewCall(bytes calldata data) external view {
        assert(router.call(data));
    }

    function goodViewTransfer(uint256 n) external view {
        assert(router.transfer(n));
    }
}

// ---- push/pop on contract/interface state vars must not be flagged ----
// These are interface method calls, not builtin array mutations. The name-only heuristic
// must not fire; resolve_member_overloads handles them precisely via mutates_state().
interface IQueue {
    function pop() external view returns (bool);
    function push(uint256 x) external view returns (bool);
}

contract AssertStateChangePushPop {
    IQueue public q;

    function goodInterfacePop() external view {
        assert(q.pop());
    }

    function goodInterfacePush(uint256 x) external view {
        assert(q.push(x));
    }
}

// ---- using-for library extension calls ----
// Solar does not yet embed Res on Member expressions for extension methods, so a dedicated
// library-scan fallback is required (fix for false negatives on using-for mutations).

library StorageLib {
    function bump(uint256[] storage arr) internal returns (bool) {
        arr.push(1);
        return true;
    }

    function peek(uint256[] storage arr) internal view returns (uint256) {
        return arr.length;
    }
}

contract AssertStateChangeUsingFor {
    using StorageLib for uint256[];

    uint256[] public items;

    // Bad: bump() writes to storage via a using-for library extension, must be flagged.
    function badLibraryExtension() external returns (bool) {
        assert(items.bump()); //~WARN: assert() argument contains a state-modifying expression
        return true;
    }

    // Good: peek() is a view extension — must NOT be flagged.
    function goodLibraryView() external view returns (bool) {
        assert(items.peek() >= 0);
        return true;
    }
}

// ---- same-arity overloads: any-mutates policy ----
// Both overloads have arity 1 but different param types. Since Solar does not resolve
// which overload was selected, we flag whenever any candidate mutates state.
interface IOverloaded {
    function check(uint256 n) external returns (bool);   // mutating
    function check(address a) external view returns (bool); // view
}

contract AssertStateChangeSameArityOverload {
    IOverloaded public o;

    // Bad: `check(uint256)` mutates state; any-mutates policy must flag this.
    function badSameArityMutating(uint256 n) external {
        assert(o.check(n)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: `check(address)` is view but `check(uint256)` mutates; still flagged.
    function badSameArityView(address a) external {
        assert(o.check(a)); //~WARN: assert() argument contains a state-modifying expression
    }
}

// ---- storage pointer aliases ----
// A local variable declared `storage` is an alias into contract storage; assignments
// through it must be treated as state mutations.
contract AssertStateChangeStorageAlias {
    uint256[] public items;
    mapping(address => uint256) public balances;

    // Good: read-only access through a storage-pointer local should not warn.
    function goodStorageArrayAliasReadOnly() external view returns (uint256) {
        uint256[] storage xs = items;
        assert((xs.length) > 0);
        return xs.length;
    }

    // Bad: assignment through a storage-pointer local to an array element.
    function badStorageArrayAliasAssign() external {
        uint256[] storage xs = items;
        assert((xs[0] = 1) > 0); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: assignment through a storage-pointer local to a mapping slot.
    function badStorageMappingAlias(address user, uint256 amt) external {
        mapping(address => uint256) storage m = balances;
        assert((m[user] = amt) > 0); //~WARN: assert() argument contains a state-modifying expression
    }
}

// ---- indexed-into contract/interface receivers (issue: false negatives) ----
// contract_id_of must resolve tokens[i] and byUser[user] by element/value type,
// not just plain Ident receivers.
interface IMutator {
    function mutate() external returns (bool);
    function peek() external view returns (bool);
}

contract AssertStateChangeIndexedContractCall {
    IMutator[] public tokens;
    mapping(address => IMutator) public byUser;

    // Bad: mutating call through an array-indexed contract variable.
    function badIndexedContractCall(uint256 i) external {
        assert(tokens[i].mutate()); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: mutating call through a mapping-indexed contract variable.
    function badMappingContractCall(address user) external {
        assert(byUser[user].mutate()); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view call through an array-indexed contract variable must NOT warn.
    function goodIndexedViewCall(uint256 i) external view {
        assert(tokens[i].peek());
    }

    // Good: view call through a mapping-indexed contract variable must NOT warn.
    function goodMappingViewCall(address user) external view {
        assert(byUser[user].peek());
    }
}

// ---- indexed interface array must not trigger builtin push/pop heuristic (issue: false positive) ----
// is_dynamic_array_or_bytes(queues[i]) must return false because queues[i] is IQueue, not a builtin array.
contract AssertStateChangeIndexedInterfaceArray {
    IQueue[] public queues;

    // Good: view interface method on an element of an interface array must NOT be flagged.
    function goodIndexedInterfacePop(uint256 idx) external view {
        assert(queues[idx].pop());
    }

    // Good: same for push — the element type is IQueue, not a builtin array.
    function goodIndexedInterfacePush(uint256 idx, uint256 x) external view {
        assert(queues[idx].push(x));
    }
}

// ---- library-fallback receiver-type guard ----
library OtherStorageLib {
    function bump(bytes storage b) internal returns (bool) {
        b.push(0x00);
        return true;
    }
}

contract AssertStateChangeUnrelatedLib {
    uint256[] public items;

    // Good: `items.bump()` has no matching extension (OtherStorageLib.bump takes bytes, not
    // uint256[]), and `uint256[]` has no member `bump()`. Must NOT be flagged.
    function goodUnrelatedLib() external view returns (bool) {
        return items.length == 0;
    }
}

// ---- struct-field receiver chain (issue: false negatives) ----
// cfg.token.transfer(...), the receiver is a struct field of contract type;
// contract_id_of must walk through the struct field to find the ContractId.
struct Config {
    IToken token;
}

contract AssertStateChangeStructFieldReceiver {
    Config public cfg;

    // Bad: mutating call through a struct-field contract variable.
    function badStructFieldCall(address to, uint256 amt) external {
        assert(cfg.token.transfer(to, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view call through a struct-field contract variable must NOT warn.
    function goodStructFieldViewCall(address who) external view {
        assert(cfg.token.balanceOf(who) > 0);
    }
}

// ---- function-call-result receiver (issue: false negatives) ----
// getToken().transfer(...), the receiver is the return value of a function call;
// contract_id_of must resolve the function's return type to find the ContractId.
contract AssertStateChangeFnReturnReceiver {
    IToken private tok;

    function getToken() internal view returns (IToken) {
        return tok;
    }

    // Bad: mutating call on the return value of a function.
    function badFnReturnCall(address to, uint256 amt) external {
        assert(getToken().transfer(to, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view call on the return value of a function must NOT warn.
    function goodFnReturnViewCall(address who) external view {
        assert(getToken().balanceOf(who) > 0);
    }
}

// ---- address-mapping index receiver (issue: false negatives) ----
// payees[user].send(...), the receiver is an element of an address mapping;
// is_address_like must handle Mapping value types, not only Array element types.
contract AssertStateChangeAddressMappingReceiver {
    mapping(address => address payable) public payees;

    // Bad: .send() on a mapping-indexed address value.
    function badMappingAddressSend(address user) external {
        assert(payees[user].send(1 ether)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: reading a mapping-indexed address value (no call) must NOT warn.
    function goodMappingAddressRead(address user) external view {
        assert(payees[user] != address(0));
    }
}

// ---- bare internal overloads with different arities ----
// Bare identifier resolution must filter overload candidates by arity; the 1-arg view overload
// should not inherit mutability from the unrelated 2-arg mutating overload.
contract AssertStateChangeBareOverloadArity {
    uint256 public counter;

    function check(uint256 n) internal view returns (bool) {
        return n == counter;
    }

    function check(uint256 n, uint256 delta) internal returns (bool) {
        counter = n + delta;
        return true;
    }

    // Good: selected overload is view, even though a same-name different-arity overload mutates.
    function goodDifferentArityBareCall(uint256 n) external view {
        assert(check(n));
    }
}

// ---- this receiver ----
// `this` is a contract-typed receiver even though it does not resolve through a variable.
contract AssertStateChangeThisReceiver {
    uint256 public counter;

    function mutate() external returns (bool) {
        counter++;
        return true;
    }

    function badThisCall() external {
        assert(this.mutate()); //~WARN: assert() argument contains a state-modifying expression
    }
}

// ---- member-call-result receiver ----
// factory.token().transfer(...), the receiver is a contract returned from a member call.
interface ITokenFactory {
    function token() external view returns (IToken);
}

contract AssertStateChangeMemberReturnReceiver {
    ITokenFactory public factory;

    // Bad: mutating call on the contract returned by a member function.
    function badMemberReturnCall(address to, uint256 amt) external {
        assert(factory.token().transfer(to, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view call on the contract returned by a member function must NOT warn.
    function goodMemberReturnViewCall(address who) external view {
        assert(factory.token().balanceOf(who) > 0);
    }
}

// ---- address struct-field receiver ----
// payee.recipient.send(...), the receiver is an address payable stored inside a struct field.
struct Payee {
    address payable recipient;
}

contract AssertStateChangeAddressStructReceiver {
    Payee public payee;

    function badStructAddressSend() external {
        assert(payee.recipient.send(1 ether)); //~WARN: assert() argument contains a state-modifying expression
    }
}
