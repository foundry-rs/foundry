// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IExternal {
    function ping() external;
}

contract MissingZeroCheck {
    address public owner;
    address payable public recipient;
    uint256 public n;

    modifier nonZero(address a) {
        require(a != address(0), "zero");
        _;
    }

    modifier doesNothing(address a) {
        _;
    }

    // SHOULD FAIL:

    function setOwner(address newOwner) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        owner = newOwner;
    }

    constructor(address initialOwner) { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        owner = initialOwner;
    }

    function pay(address payable to) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        to.transfer(1);
    }

    function lowLevel(address payable to, bytes calldata data) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        (bool ok,) = to.call(data);
        require(ok);
    }

    function withUselessModifier(address a) external doesNothing(a) { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        owner = a;
    }

    function setOwnerViaAlias(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        address tmp = a;
        owner = tmp;
    }

    function setOwnerViaReassign(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        address tmp;
        tmp = a;
        owner = tmp;
    }

    function setOwnerViaCast(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        owner = address(uint160(a));
    }

    function payViaAlias(address payable a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        address payable tmp = a;
        tmp.transfer(1);
    }

    // Only `b` should be flagged; `a` is guarded.
    function mixedParams(address a, address b) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        require(a != address(0));
        owner = a;
        recipient = payable(b);
    }

    // Same param feeds two sinks: should produce a single diagnostic.
    function bothSinks(address payable a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        recipient = a;
        a.transfer(1);
    }

    function ternaryAlias(address a, bool flag) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        address tmp = flag ? a : address(0);
        owner = tmp;
    }

    function payableWrap(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        payable(a).transfer(1);
    }

    // Modifier called with an expression, not a direct ident: we cannot prove the guard
    // applies, so we should still flag.
    function modifierWithExpr(address a) external nonZero(addrIdentity(a)) { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        owner = a;
    }

    function delegateCallSink(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        (bool ok,) = a.delegatecall("");
        require(ok);
    }

    function sendSinkStmt(address payable a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        a.send(1);
    }

    function sendSinkDecl(address payable a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        bool ok = a.send(1);
        require(ok);
    }

    function multiHopTaint(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        address x = a;
        address y = x;
        owner = y;
    }

    function guardAfterSink(address a) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        owner = a;
        require(a != address(0));
    }

    function guardOnOneBranch(address a, bool flag) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        if (flag) {
            require(a != address(0));
        }
        owner = a;
    }

    function guardInForLoop(address a, uint256 n) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        for (uint256 i = 0; i < n; i++) {
            require(a != address(0));
        }
        owner = a;
    }

    function guardInWhileLoop(address a, bool flag) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        while (flag) {
            require(a != address(0));
            flag = false;
        }
        owner = a;
    }

    function guardInTryClause(address a, address payable target) external { //~WARN: address parameter is used in a state write or value transfer without a zero-address check
        try IExternal(target).ping() {
            require(a != address(0));
        } catch {
            require(a != address(0));
        }
        owner = a;
    }

    // SHOULD PASS:

    function setOwnerGuarded(address newOwner) external {
        require(newOwner != address(0), "zero");
        owner = newOwner;
    }

    function setOwnerIfGuarded(address newOwner) external {
        if (newOwner == address(0)) revert();
        owner = newOwner;
    }

    function setOwnerAssertGuarded(address newOwner) external {
        assert(newOwner != address(0));
        owner = newOwner;
    }

    function setOwnerWithModifier(address newOwner) external nonZero(newOwner) {
        owner = newOwner;
    }

    function setN(uint256 v) external {
        n = v;
    }

    function viewer(address a) external view returns (address) {
        return a;
    }

    function pureFn(address a) external pure returns (address) {
        return a;
    }

    function internalHelper(address a) internal {
        owner = a;
    }

    function callsHelper(address a) external {
        require(a != address(0));
        internalHelper(a);
    }

    function setOwnerViaAliasGuarded(address a) external {
        require(a != address(0));
        address tmp = a;
        owner = tmp;
    }

    function privateHelper2(address a) private {
        owner = a;
    }

    event Deposit(address indexed from);
    error ZeroAddress();

    function emitOnly(address a) external {
        emit Deposit(a);
    }

    function guardViaCustomRevert(address a) external {
        if (a == address(0)) revert ZeroAddress();
        owner = a;
    }

    function noSinkJustPassthrough(address a) external returns (address) {
        return a;
    }

    function addrIdentity(address x) internal pure returns (address) {
        return x;
    }

    function staticCallOnly(address a) external {
        (bool ok,) = a.staticcall("");
        require(ok);
    }

    // Symmetric guard on both branches: universally checked.
    function guardOnBothBranches(address a, bool flag) external {
        if (flag) {
            require(a != address(0));
        } else {
            require(a != address(0));
        }
        owner = a;
    }

    // Inner zero-check guards `a` for the rest of the enclosing branch via early revert.
    function nestedGuardWithRevert(address a, bool flag) external {
        if (flag) {
            if (a == address(0)) revert();
            owner = a;
        }
    }
}
