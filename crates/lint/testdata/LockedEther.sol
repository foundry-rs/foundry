//@compile-flags: --only-lint locked-ether

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IERC20 {
    function transfer(address, uint256) external returns (bool);
}

interface IOneArgTransfer {
    function transfer(uint256 amount) external;
    function send(uint256 amount) external;
}

// SHOULD FAIL:

contract LockedReceive { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}
}

contract LockedFallback { //~WARN: contract can receive ETH but has no mechanism to send it out
    fallback() external payable {}
}

contract LockedPayableFn { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}
}

contract LockedPayableCtor { //~WARN: contract can receive ETH but has no mechanism to send it out
    constructor() payable {}
}

contract LockedZeroValue { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function noop(address payable to) external {
        to.transfer(0);
        bool ok = to.send(0);
        ok;
        (bool s,) = to.call{value: 0}("");
        s;
    }
}

contract LockedTokenOnly { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function rescueToken(address token, address to, uint256 amount) external {
        IERC20(token).transfer(to, amount);
    }
}

contract LockedOneArgTokenTransfer { //~WARN: contract can receive ETH but has no mechanism to send it out
    IOneArgTransfer token;

    function deposit() external payable {}

    function pay(uint256 amount) external {
        token.transfer(amount);
        token.send(amount);
    }
}

contract Helper {
    function pay(address payable to, uint256 amount) external {
        to.transfer(amount);
    }
}

contract LockedNotInherited { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}
}

contract Child { //~WARN: contract can receive ETH but has no mechanism to send it out
    constructor() payable {}
}

// Unreachable internal helpers don't count as exits.
contract LockedUnreachableInternal { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function _sweep(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract LockedUnreachablePrivate { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function _drain(address payable to) private {
        selfdestruct(to);
    }
}

// Overload resolution: the dead 0-arg `_do()` overload must not be followed.
abstract contract OverloadBase {
    function _do() internal {
        payable(msg.sender).transfer(1 ether);
    }
    function _do(uint256) internal {}
}

contract LockedSuperOverload is OverloadBase { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function f() external {
        super._do(1);
    }
}

library OverloadLib {
    function pay() internal {
        payable(msg.sender).transfer(1 ether);
    }
    function pay(uint256) internal {}
}

contract LockedLibraryOverload { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function f() external {
        OverloadLib.pay(1);
    }
}

// Overridden base implementation is no longer in the dispatch table.
abstract contract OverriddenBaseWithExit {
    function withdraw() external virtual {
        payable(msg.sender).transfer(address(this).balance);
    }
}

contract LockedOverriddenWithdraw is OverriddenBaseWithExit { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function withdraw() external override {}
}

// Same dispatch rule for `receive`: only the most-derived one is reachable.
abstract contract BaseReceiveWithExit {
    receive() external payable virtual {
        payable(msg.sender).transfer(msg.value);
    }
}

contract LockedOverriddenReceive is BaseReceiveWithExit { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable override {}
}

// Constructor exits don't cover runtime ETH inflows.
contract LockedCtorExitWithRuntimeReceive { //~WARN: contract can receive ETH but has no mechanism to send it out
    constructor() payable {
        payable(msg.sender).transfer(msg.value);
    }

    receive() external payable {}
}

// A send followed by an unconditional revert in the same function body never commits.
contract LockedExitThenRevert { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function nope(address payable to, uint256 amount) external {
        to.transfer(amount);
        revert();
    }
}

// The exit lives in an always-reverting helper, so it can never run on-chain.
contract LockedExitInRevertingHelper { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function entry(address payable to, uint256 amount) external {
        _send(to, amount);
    }

    function _send(address payable to, uint256 amount) internal {
        to.transfer(amount);
        revert();
    }
}

// SHOULD PASS:

contract OkTransfer {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        to.transfer(amount);
    }
}

contract OkSend {
    receive() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        bool ok = to.send(amount);
        require(ok);
    }
}

contract OkCallWithValue {
    function deposit() external payable {}

    function withdraw(address to, uint256 amount) external {
        (bool ok,) = to.call{value: amount}("");
        require(ok);
    }
}

contract OkSelfdestruct {
    function deposit() external payable {}

    function close(address payable to) external {
        selfdestruct(to);
    }
}

contract OkNewWithValue {
    function deposit() external payable {}

    // OK even though `Child` is itself flagged: `new C{value: x}()` is the exit here.
    function spawn(uint256 amount) external {
        new Child{value: amount}();
    }
}

abstract contract Withdrawable {
    function _withdraw(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkInheritedWithdraw is Withdrawable {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        _withdraw(to, amount);
    }
}

contract OkTransitive {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        _doSend(to, amount);
    }

    function _doSend(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

// `this.<fn>(...)` member-call dispatch.
contract OkExternalSelfCall {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        this._doSend(to, amount);
    }

    function _doSend(address payable to, uint256 amount) external {
        to.transfer(amount);
    }
}

// Inline assembly: opaque, bails when reached.
contract OkAssemblyExit {
    function deposit() external payable {}

    function withdraw(address to, uint256 amount) external {
        assembly {
            let ok := call(gas(), to, amount, 0, 0, 0, 0)
            if iszero(ok) { revert(0, 0) }
        }
    }
}

contract OkAssemblySelfdestruct {
    function deposit() external payable {}

    function close(address to) external {
        assembly {
            selfdestruct(to)
        }
    }
}

contract OkDelegatecall {
    function deposit() external payable {}

    function delegate(address impl, bytes calldata data) external {
        (bool ok,) = impl.delegatecall(data);
        require(ok);
    }
}

// Payable surface that always reverts.
contract OkReceiveAlwaysReverts {
    receive() external payable {
        revert();
    }
}

contract OkFallbackAlwaysReverts {
    fallback() external payable {
        revert("disabled");
    }
}

contract OkPayableFnAlwaysReverts {
    function deposit() external payable {
        revert("disabled");
    }
}

contract OkPayableRequireFalse {
    function deposit() external payable {
        require(false, "disabled");
    }
}

contract OkPayableAssertFalse {
    function deposit() external payable {
        assert(false);
    }
}

// Modifier always reverts before `_`.
contract OkPayableModifierReverts {
    modifier disabled() {
        revert("disabled");
        _;
    }

    function deposit() external payable disabled {}
}

// Modifier always reverts after `_`.
contract OkPayableModifierRevertsAfter {
    modifier disabledAfter() {
        _;
        revert("disabled");
    }

    function deposit() external payable disabledAfter {}
}

// `super.<m>(...)` member-call dispatch.
abstract contract SuperWithdrawBase {
    function _doSend(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkSuperCall is SuperWithdrawBase {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        super._doSend(to, amount);
    }
}

// `super.<m>(...)` whose call site lives in a base contract: resolution must use the
// call-site contract's own linearization, otherwise it walks past the real exit in
// `LeafSuperExit` and hits `BaseSuperCallSite`'s empty override instead.
abstract contract LeafSuperExit {
    function _exit(address payable to) internal virtual {
        to.transfer(address(this).balance);
    }
}

abstract contract BaseSuperCallSite is LeafSuperExit {
    function _exit(address payable) internal virtual override {}

    function withdraw(address payable to) external {
        super._exit(to);
    }
}

contract OkSuperFromBase is BaseSuperCallSite {
    receive() external payable {}
}

// Same-arity overloads differing only by parameter type. Dispatch must pick the
// `uint256` variant; following the `address payable` overload would let its
// unrelated exit silence the lint.
abstract contract OverloadByType {
    function _send(uint256) internal {}
    function _send(address payable to) internal {
        to.transfer(1 wei);
    }
}

contract LockedSameArityOverload is OverloadByType { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function withdraw(uint256 amount) external {
        _send(amount);
    }
}

// Same-arity overloads disambiguated by named-arg parameter set.
contract OkNamedArgOverload {
    function deposit() external payable {}

    function _send(uint256, address payable) internal {}
    function _send(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }

    function withdraw(address payable to, uint256 amount) external {
        _send({to: to, amount: amount});
    }
}

// Self-sends keep ETH inside the contract; they must not count as exits.
contract LockedThisCallWithValue { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function deposit() external payable {}

    function loop(uint256 x) external {
        this.deposit{value: x}();
    }
}

contract LockedAddressThisCallWithValue { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function loop(uint256 x) external {
        (bool ok,) = address(this).call{value: x}("");
        require(ok);
    }
}

contract LockedTransferToSelf { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function loop(uint256 x) external {
        payable(address(this)).transfer(x);
    }
}

contract LockedSelfdestructToSelf { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}

    function nuke() external {
        selfdestruct(payable(address(this)));
    }
}

// Receivers reached through struct fields, mapping/array elements, function returns,
// and ternaries are valid `address payable` values and must be recognized as exits.
contract OkStructFieldReceiver {
    struct Vault {
        address payable owner;
    }

    Vault v;

    function deposit() external payable {}

    function withdraw(uint256 x) external {
        v.owner.transfer(x);
    }
}

contract OkMappingReceiver {
    mapping(uint256 => address payable) recipients;

    function deposit() external payable {}

    function withdraw(uint256 i, uint256 x) external {
        recipients[i].transfer(x);
    }
}

contract OkArrayReceiver {
    address payable[] recipients;

    function deposit() external payable {}

    function withdraw(uint256 i, uint256 x) external {
        recipients[i].transfer(x);
    }
}

contract OkReturnedAddressReceiver {
    address payable treasury;

    function deposit() external payable {}

    function _getTreasury() internal view returns (address payable) {
        return treasury;
    }

    function withdraw(uint256 x) external {
        _getTreasury().transfer(x);
    }
}

contract OkTernaryReceiver {
    address payable a;
    address payable b;

    function deposit() external payable {}

    function withdraw(bool which, uint256 x) external {
        (which ? a : b).transfer(x);
    }
}

// `.send` and low-level `{value:}` on compound receivers reuse the same type-inference
// path as `.transfer`; pin both shapes to lock them in.
contract OkMappingSend {
    mapping(uint256 => address payable) recipients;

    function deposit() external payable {}

    function withdraw(uint256 i, uint256 x) external {
        bool ok = recipients[i].send(x);
        require(ok);
    }
}

contract OkCallWithValueThroughReturn {
    address payable treasury;

    function deposit() external payable {}

    function _treasury() internal view returns (address payable) {
        return treasury;
    }

    function withdraw(uint256 x) external {
        (bool ok,) = _treasury().call{value: x}("");
        require(ok);
    }
}

// `super.<m>(...)` whose call site lives in a base that is part of a diamond. The
// derived contract's linearization places `DiamondRightBranch` before
// `DiamondLeftBranch`, so resolving `super` from the most-derived linearization would
// hit `DiamondRightBranch`'s empty override; only the call-site contract's own
// linearization reaches `DiamondRoot._exit`.
abstract contract DiamondRoot {
    function _exit(address payable to) internal virtual {
        to.transfer(1 wei);
    }
}

abstract contract DiamondLeftBranch is DiamondRoot {
    function _exit(address payable to) internal virtual override {
        super._exit(to);
    }
}

abstract contract DiamondRightBranch is DiamondRoot {
    function _exit(address payable to) internal virtual override {}
}

contract OkDiamondSuperFromBase is DiamondLeftBranch, DiamondRightBranch {
    receive() external payable {}

    function _exit(address payable to)
        internal
        override(DiamondLeftBranch, DiamondRightBranch)
    {
        DiamondLeftBranch._exit(to);
    }

    function withdraw(address payable to) external {
        _exit(to);
    }
}

// `using for` and function-pointer dispatch route through bindings Solar's HIR doesn't
// expose; treat those calls as opaque so the lint doesn't flag valid withdraw paths.
library SendViaLib {
    function sweep(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkUsingFor {
    using SendViaLib for address payable;

    function deposit() external payable {}

    function withdraw(address payable to, uint256 x) external {
        to.sweep(x);
    }
}

contract OkFunctionPointer {
    function _doSend(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }

    function(address payable, uint256) internal sender = _doSend;

    function deposit() external payable {}

    function withdraw(address payable to, uint256 x) external {
        sender(to, x);
    }
}

// Receiver produced by a method call on a contract-typed value: the inner call's return
// type must propagate so `.transfer` on it is recognized as an exit.
contract Registry {
    address payable private _owner;
    function getOwner() external view returns (address payable) { return _owner; }
}

contract OkMemberCallReturnReceiver {
    Registry registry;

    function deposit() external payable {}

    function withdraw(uint256 x) external {
        registry.getOwner().transfer(x);
    }
}

// `Lib.<fn>(...)` member-call dispatch.
library SendLib {
    function pay(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkLibraryCall {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        SendLib.pay(to, amount);
    }
}

// Nested library helpers should not be rebound through same-named functions on the
// consuming contract.
library SendLibNested {
    function pay(address payable to, uint256 amount) internal {
        _send(to, amount);
    }

    function _send(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkNestedLibraryCall {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        SendLibNested.pay(to, amount);
    }

    function _send(address payable, uint256) internal {}
}

contract OkReferenceTypeArg {
    function deposit() external payable {}

    function withdraw(address payable to, uint256[] memory amounts) external {
        _send(to, amounts);
    }

    function _send(address payable to, uint256[] memory) internal {
        to.transfer(1 wei);
    }
}

// A non-payable derived ctor rejects deployment value even if a base ctor is payable.
abstract contract PayableBaseCtor { //~WARN: contract can receive ETH but has no mechanism to send it out
    constructor() payable {}
}

contract OkNonPayableChildOfPayableBase is PayableBaseCtor {
    constructor() PayableBaseCtor() {}
}

// No explicit ctor: synthesized non-payable rejects deployment value.
contract OkImplicitCtorOfPayableBase is PayableBaseCtor {}

// Inherited external follows a virtual internal hook; the leaf override removes the exit.
abstract contract VirtualExitBase {
    function withdraw(address payable to) external { _exit(to); }
    function _exit(address payable to) internal virtual { to.transfer(1 wei); }
}

contract LockedOverriddenVirtualHook is VirtualExitBase { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}
    function _exit(address payable) internal override {}
}

// A `return` on one branch keeps the function reachable even if the other branch reverts,
// so the payable inflow still counts and the contract is locked.
contract LockedReceiveReturnBeforeRevert { //~WARN: contract can receive ETH but has no mechanism to send it out
    bool open;
    receive() external payable {
        if (open) return;
        revert();
    }
}

// `{value:...}` on a contract/interface cast of self stays in this contract; not an exit.
interface ISelfSink { function deposit() external payable; }

contract LockedSelfSendViaInterfaceCast is ISelfSink { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}
    function deposit() external payable override {}
    function loop(uint256 x) external {
        ISelfSink(address(this)).deposit{value: x}();
    }
}

contract NotPayable {
    function ping() external pure returns (bool) {
        return true;
    }
}

library OkLib {
    function helper() internal pure returns (uint256) {
        return 1;
    }
}

interface IOk {
    function payme() external payable;
}
