//@compile-flags: --only-lint arbitrary-send-eth

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

interface IThing {
    function ping() external payable;
}

interface IRegistry {
    function recipient() external view returns (address payable);
}

interface IOverloaded {
    function recipient(uint256 x) external view returns (uint256);
    function recipient() external view returns (address payable);
}

interface IOverloadedSameArity {
    function recipient(uint256 x) external view returns (uint256);
    function recipient(address x) external view returns (address payable);
}

library Lib {
    function pay(address payable dest, uint256 amt) internal { dest.transfer(amt); }
}

library SenderLib {
    function self(address a) internal pure returns (address) { return a; }
}

library Address {
    function sendValue(address payable to, uint256 amount) internal { (bool ok,) = to.call{value: amount}(""); require(ok); }
    function functionCallWithValue(address target, bytes memory data, uint256 value) internal returns (bytes memory) {
        (bool ok, bytes memory ret) = target.call{value: value}(data);
        require(ok);
        return ret;
    }
    function functionCallWithValue(
        address target,
        bytes memory data,
        uint256 value,
        string memory errorMessage
    ) internal returns (bytes memory) {
        (bool ok, bytes memory ret) = target.call{value: value}(data);
        require(ok, errorMessage);
        return ret;
    }
}

library SafeTransferLib {
    function safeTransferETH(address to, uint256 amount) internal { (bool ok,) = to.call{value: amount}(""); require(ok); }
    function forceSafeTransferETH(address to, uint256 amount) internal { (bool ok,) = to.call{value: amount}(""); require(ok); }
    function forceSafeTransferETH(address to, uint256 amount, uint256 gasStipend) internal { (bool ok,) = to.call{value: amount, gas: gasStipend}(""); require(ok); }
    function safeTransferAllETH(address to) internal { (bool ok,) = to.call{value: address(this).balance}(""); require(ok); }
    function forceSafeTransferAllETH(address to) internal { (bool ok,) = to.call{value: address(this).balance}(""); require(ok); }
    function forceSafeTransferAllETH(address to, uint256 gasStipend) internal { (bool ok,) = to.call{value: address(this).balance, gas: gasStipend}(""); require(ok); }
    function trySafeTransferETH(address to, uint256 amount, uint256 gasStipend) internal returns (bool success) { (success,) = to.call{value: amount, gas: gasStipend}(""); }
    function trySafeTransferAllETH(address to, uint256 gasStipend) internal returns (bool success) { (success,) = to.call{value: address(this).balance, gas: gasStipend}(""); }
    function safeMoveETH(address to, uint256 amount) internal returns (address vault) {
        (bool ok,) = to.call{value: amount}("");
        require(ok);
        vault = to;
    }
}

contract NonLibraryBase {
    function safeTransferETH(address, uint256) internal pure {}
}

contract ArbitrarySendEth {
    using Lib for address payable;
    using Address for address payable;
    using Address for address;
    using SafeTransferLib for address;
    using SenderLib for address;
    struct Cfg {
        address payable beneficiary;
    }
    address public mutableOwner;
    address public immutable trustedOwner;
    address public constant TREASURY = 0x000000000000000000000000000000000000dEaD;
    IThing public immutable trustedThing;
    IERC20 public token;
    address payable[] public recipients;
    mapping(address => address payable) public delegates;
    address[] public admins;
    mapping(address => uint256) public adminIndex;
    Cfg public cfg;
    constructor(address _owner, IThing _thing, address payable seed, uint256 amt) payable { trustedOwner = _owner; trustedThing = _thing; seed.transfer(amt); }
    function refundCaller() external payable { payable(msg.sender).transfer(msg.value); }
    function payTxOrigin(uint256 amt) external { payable(tx.origin).transfer(amt); }
    function selfTopUp(uint256 amt) external { payable(address(this)).transfer(amt); }
    function selfCall(bytes calldata data, uint256 amt) external returns (bool ok) { (ok,) = address(this).call{value: amt}(data); }
    function selfInterfaceCall(uint256 amt) external { IThing(address(this)).ping{value: amt}(); }
    function noValueCall(address t, bytes calldata data) external returns (bool ok) { (ok,) = t.call(data); }
    function zeroValueCall(address t, bytes calldata data) external returns (bool ok) { (ok,) = t.call{value: 0}(data); }
    function zeroTransfer(address payable t) external { t.transfer(0); }
    function payImmutableOwner(uint256 amt) external { payable(trustedOwner).transfer(amt); }
    function payImmutableThing(uint256 amt) external { trustedThing.ping{value: amt}(); }
    function payConstantTreasury(uint256 amt) external { payable(TREASURY).transfer(amt); }
    function payLiteralAddress(uint256 amt) external { payable(0x000000000000000000000000000000000000dEaD).transfer(amt); }
    function payErc20Transfer(address to, uint256 amt) external { token.transfer(to, amt); }
    function selfDestructHere() external { selfdestruct(payable(address(this))); }
    function badTransfer(address payable to, uint256 amt) external { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSend(address payable to, uint256 amt) external {
        bool ok = to.send(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
        ok;
    }
    function badCall(address to, uint256 amt, bytes calldata data) external returns (bool ok) { (ok,) = to.call{value: amt}(data); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badInterfaceCall(IThing t, uint256 amt) external { t.ping{value: amt}(); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSelfDestruct(address payable to) external { selfdestruct(to); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badPayMutableStorage(uint256 amt) external { payable(mutableOwner).transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function setOwner(address newOwner) external { mutableOwner = newOwner; }
    function badTernaryDest(address payable a, address payable b, bool flag, uint256 amt) external { (flag ? a : b).transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badArrayDest(uint256 i, uint256 amt) external { recipients[i].transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badMappingDest(address k, uint256 amt) external { delegates[k].transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badStructDest(uint256 amt) external { cfg.beneficiary.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSendValueStatic(address payable to, uint256 amt) external { Address.sendValue(to, amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSendValueUsingFor(address payable to, uint256 amt) external { to.sendValue(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSafeTransferETH(address to, uint256 amt) external { SafeTransferLib.safeTransferETH(to, amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badForceSafeTransferETH(address to, uint256 amt) external { to.forceSafeTransferETH(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSafeTransferAllETHUsingFor(address to) external { to.safeTransferAllETH(); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSafeTransferAllETHStatic(address to) external { SafeTransferLib.safeTransferAllETH(to); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badForceSafeTransferAllETHWithGas(address to, uint256 gasStipend) external { to.forceSafeTransferAllETH(gasStipend); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badFunctionCallWithValue(address to, bytes calldata data, uint256 amt) external { to.functionCallWithValue(data, amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badFunctionCallWithValueStaticWithErr(address to, bytes calldata data, uint256 amt) external { Address.functionCallWithValue(to, data, amt, "boom"); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badFunctionPointer(function() external payable f, uint256 amt) external { f{value: amt}(); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function aliasedSenderOk(uint256 amt) external { address payable to = payable(msg.sender); to.transfer(amt); }
    function ifGuardedOk(address payable to, uint256 amt) external {
        if (to == payable(msg.sender)) {
            to.transfer(amt);
        }
    }
    function requireGuardedOk(address payable to, uint256 amt) external { require(to == payable(msg.sender), "bad to"); to.transfer(amt); }
    function revertGuardedOk(address payable to, uint256 amt) external {
        if (to != payable(msg.sender)) {
            revert();
        }
        to.transfer(amt);
    }
    function ternarySafeOk(uint256 amt, bool flag) external { address payable t = flag ? payable(msg.sender) : payable(address(this)); t.transfer(amt); }
    function interfaceCastValidatedOk(address x, uint256 amt) external { require(x == msg.sender); IThing(x).ping{value: amt}(); }
    function reassignedAfterCheckBad(address payable to, address payable other, uint256 amt) external {
        require(to == payable(msg.sender));
        to = other; // kills the safe-fact
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function elseBranchBad(address payable to, uint256 amt) external {
        if (to == payable(msg.sender)) {
            to.transfer(amt); // safe via positive fact
        } else {
            to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
        }
    }
    function unreachableAfterRevertOk(address payable to, uint256 amt) external {
        revert();
        to.transfer(amt); // unreachable; must NOT flag
    }
    modifier onlyOwner() { require(msg.sender == trustedOwner, "not owner"); _; }
    modifier onlyOwnerIfRevert() {
        if (msg.sender != trustedOwner) {
            revert();
        }
        _;
    }
    modifier onlyEOA() { require(msg.sender.code.length == 0, "not eoa"); _; }
    modifier nonZeroSender() { require(msg.sender != address(0)); _; }
    modifier whenNotPaused() { require(block.timestamp > 0, "paused"); _; }
    function withdrawAnywhere(address payable to, uint256 amt) external onlyOwner { to.transfer(amt); }
    function withdrawAnywhereRevert(address payable to, uint256 amt) external onlyOwnerIfRevert { to.transfer(amt); }
    function pausedButOpenBad(address payable to, uint256 amt) external whenNotPaused { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function eoaOnlyBad(address payable to, uint256 amt) external onlyEOA { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function nonZeroSenderBad(address payable to, uint256 amt) external nonZeroSender { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier checkSender(address payable who) { require(who == payable(msg.sender), "not sender"); _; }
    function paramSafeViaModifierOk(address payable to, uint256 amt) external checkSender(to) { to.transfer(amt); }
    function paramSafeButOtherBad(address payable to, address payable other, uint256 amt) external checkSender(to) { other.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier maybeOwner(bool b) { if (b) require(msg.sender == trustedOwner); _; }
    function maybeOwnerBad(address payable to, uint256 amt, bool b) external maybeOwner(b) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    struct Guard {
        address principal;
    }
    modifier fakeGuard(Guard memory g) { require(msg.sender == g.principal, "bad"); _; }
    function fakeGuardBad(address payable to, uint256 amt, Guard memory g) external fakeGuard(g) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function callSiteUnchecked(address payable to, uint256 amt) external { Lib.pay(to, amt); }
    modifier tautologicalSender() { require(msg.sender == msg.sender); _; }
    function tautologicalSenderBad(address payable to, uint256 amt) external tautologicalSender { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier txOriginOnly() { require(msg.sender == tx.origin); _; }
    function txOriginOnlyBad(address payable to, uint256 amt) external txOriginOnly { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function _msgSender() internal view returns (address) { return msg.sender; }
    modifier onlyOwnerViaHelper() { require(_msgSender() == trustedOwner); _; }
    function withdrawAnywhereHelper(address payable to, uint256 amt) external onlyOwnerViaHelper { to.transfer(amt); }
    modifier ifEnabledElseRevert(bool enabled) {
        if (enabled) {
            require(msg.sender == trustedOwner);
        } else {
            revert();
        }
        _;
    }
    function withdrawIfEnabled(address payable to, uint256 amt, bool enabled) external ifEnabledElseRevert(enabled) { to.transfer(amt); }
    function payAddressZeroOk(uint256 amt) external { payable(address(0)).transfer(amt); }
    modifier tautologicalSenderMemberCall() { require(msg.sender == msg.sender.self()); _; }
    function tautologicalSenderMemberCallBad(address payable to, uint256 amt) external tautologicalSenderMemberCall { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function _txOrigin() internal view returns (address) { return tx.origin; }
    modifier txOriginHelperOnly() { require(msg.sender == _txOrigin()); _; }
    function txOriginHelperOnlyBad(address payable to, uint256 amt) external txOriginHelperOnly { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function numericAmountGuardStillBad(address payable to, uint256 amt) external {
        require(amt == 1);
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier aliasIndexedAdmin(uint256 i) { uint256 j = i; require(msg.sender == admins[j]); _; }
    function aliasIndexedAdminBad(address payable to, uint256 amt, uint256 i) external aliasIndexedAdmin(i) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier nestedSenderIndexedAdmin() { require(msg.sender == admins[adminIndex[msg.sender]]); _; }
    function nestedSenderIndexedAdminBad(address payable to, uint256 amt) external nestedSenderIndexedAdmin { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function _senderIndex() internal view returns (uint256) { return adminIndex[msg.sender]; }
    modifier hiddenSenderIndexedAdmin() { require(msg.sender == admins[_senderIndex()]); _; }
    function hiddenSenderIndexedAdminBad(address payable to, uint256 amt) external hiddenSenderIndexedAdmin { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function _calldataIndex() internal pure returns (uint256) { return uint256(uint8(msg.data[4])); }
    modifier pureCalldataIndexedAdmin() { require(msg.sender == admins[_calldataIndex()]); _; }
    function pureCalldataIndexedAdminBad(address payable to, uint256 amt) external pureCalldataIndexedAdmin { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function _calldataOwner() internal pure returns (address) { return address(uint160(uint256(bytes32(msg.data[4:36])))); }
    modifier calldataOwnerGuard() { require(msg.sender == _calldataOwner()); _; }
    function calldataOwnerGuardBad(address payable to, uint256 amt) external calldataOwnerGuard { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier assignedParamGuard(address payable who) { who = payable(msg.sender); _; }
    function assignedParamGuardBad(address payable to, uint256 amt) external assignedParamGuard(to) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier assignedParamCondGuard(address payable who) {
        if ((who = payable(msg.sender)) == payable(msg.sender)) {}
        _;
    }
    function assignedParamCondGuardBad(address payable to, uint256 amt) external assignedParamCondGuard(to) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier modifierSends(address payable to, uint256 amt) {
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
        _;
    }
    function modifierSinkOk(address payable to, uint256 amt)
        external
        modifierSends(to, amt)
    {}
    function _maybeCheckOwner(bool skip) internal view { if (skip) return; require(msg.sender == trustedOwner); }
    modifier maybeChecked(bool skip) { _maybeCheckOwner(skip); _; }
    function helperEarlyReturnBad(address payable to, uint256 amt, bool skip) external maybeChecked(skip) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function memberReturnTransferBad(IRegistry r, uint256 amt) external { r.recipient().transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function contractLocalFromImmutableOk(uint256 amt) external { IThing t = trustedThing; t.ping{value: amt}(); }
    function nestedUnreachableOk(address payable to, uint256 amt) external {
        if (true) {
            revert();
            to.transfer(amt); // unreachable; must NOT flag
        }
    }
    function inlineCallerGuardOk(address payable to, uint256 amt) external { require(msg.sender == trustedOwner); to.transfer(amt); }
    function inlineCallerGuardRevertOk(address payable to, uint256 amt) external { if (msg.sender != trustedOwner) revert(); to.transfer(amt); }
    function inlineCallerGuardOrderBad(address payable to, uint256 amt) external {
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
        require(msg.sender == trustedOwner);
    }
    modifier guardedModifierSendOk(address payable to, uint256 amt) { require(msg.sender == trustedOwner); to.transfer(amt); _; }
    function guardedModifierSendOkCaller(address payable to, uint256 amt)
        external
        guardedModifierSendOk(to, amt)
    {}
    modifier sinkBeforeGuardBad(address payable to, uint256 amt) {
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
        require(msg.sender == trustedOwner);
        _;
    }
    function sinkBeforeGuardBadCaller(address payable to, uint256 amt)
        external
        sinkBeforeGuardBad(to, amt)
    {}
    function overloadedRecipientBad(IOverloaded r, uint256 amt) external { r.recipient().transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function overloadedSameArityRecipientBad(
        IOverloadedSameArity r,
        address who,
        uint256 amt
    ) external {
        r.recipient(who).transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function _consume(bool) internal pure returns (bool) { return true; }
    modifier consumeArg(bool ok) { require(ok); _; }
    function modifierArgSinkBad(address payable to, uint256 amt)
        external
        onlyOwner
        consumeArg(_consume(to.send(amt))) //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    {}
    function conditionalInlineGuardBad(address payable to, uint256 amt, bool b) external {
        if (b) require(msg.sender == trustedOwner);
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    modifier guardedSuffixSendOk(address payable to, uint256 amt) { require(msg.sender == trustedOwner); _; to.transfer(amt); }
    function guardedSuffixSendOkCaller(address payable to, uint256 amt)
        external
        guardedSuffixSendOk(to, amt)
    {}
    modifier suffixSendBad(address payable to, uint256 amt) {
        _;
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function suffixSendBadCaller(address payable to, uint256 amt)
        external
        suffixSendBad(to, amt)
    {}
    function badForceSafeTransferETHStaticWithGas(address to, uint256 amt, uint256 gasStipend) external { SafeTransferLib.forceSafeTransferETH(to, amt, gasStipend); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badForceSafeTransferETHUsingForWithGas(address to, uint256 amt, uint256 gasStipend) external { to.forceSafeTransferETH(amt, gasStipend); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badNamedSafeTransferETHStatic(address to, uint256 amt) external { SafeTransferLib.safeTransferETH({amount: amt, to: to}); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badNamedFunctionCallWithValueStatic(address to, bytes calldata data, uint256 amt) external { Address.functionCallWithValue({value: amt, target: to, data: data}); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function okNamedSafeTransferETHStaticZero(address to) external { SafeTransferLib.safeTransferETH({amount: 0, to: to}); }
    function badTrySafeTransferETHStatic(address to, uint256 amt, uint256 gas_) external { SafeTransferLib.trySafeTransferETH(to, amt, gas_); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badTrySafeTransferETHUsingFor(address to, uint256 amt, uint256 gas_) external { to.trySafeTransferETH(amt, gas_); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badTrySafeTransferAllETHStatic(address to, uint256 gas_) external { SafeTransferLib.trySafeTransferAllETH(to, gas_); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badTrySafeTransferAllETHUsingFor(address to, uint256 gas_) external { to.trySafeTransferAllETH(gas_); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSafeMoveETHStatic(address to, uint256 amt) external { SafeTransferLib.safeMoveETH(to, amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function badSafeMoveETHUsingFor(address to, uint256 amt) external { to.safeMoveETH(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract NonLibraryStaticOk is NonLibraryBase {
    function ok(address to, uint256 amt) external pure { NonLibraryBase.safeTransferETH(to, amt); }
}

interface IBaseRecipient {
    function recipient() external view returns (address payable);
}

interface IChildRecipient is IBaseRecipient {}

contract InheritedRecipient {
    function inheritedRecipientBad(IChildRecipient r, uint256 amt) external { r.recipient().transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract DisjunctiveCallerRestriction {
    address public immutable owner;
    address public immutable guardian;
    address public immutable backup;
    uint256 public cap;
    constructor(address _owner, address _guardian, address _backup) { owner = _owner; guardian = _guardian; backup = _backup; }
    modifier onlyOwnerOrGuardian() { require(msg.sender == owner || msg.sender == guardian); _; }
    modifier onlyOwnerOrGuardianOrBackup() {
        require(
            msg.sender == owner || msg.sender == guardian || msg.sender == backup
        );
        _;
    }
    modifier onlyOwnerOrGuardianIfRevert() { if (msg.sender != owner && msg.sender != guardian) revert(); _; }
    function withdrawOwnerOrGuardianOk(address payable to, uint256 amount) external onlyOwnerOrGuardian { to.transfer(amount); }
    function withdrawOwnerOrGuardianOrBackupOk(address payable to, uint256 amount) external onlyOwnerOrGuardianOrBackup { to.transfer(amount); }
    function withdrawIfRevertOk(address payable to, uint256 amount) external onlyOwnerOrGuardianIfRevert { to.transfer(amount); }
    function mixedDisjunctBad(address payable to, uint256 amount) external {
        require(msg.sender == owner || amount < cap);
        to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function inlineDestinationDisjunctionOk(address payable to, uint256 amount) external { require(to == payable(msg.sender) || to == payable(address(this))); to.transfer(amount); }
    function inlineDestinationDisjunctionIfRevertOk(address payable to, uint256 amount) external { if (to != payable(msg.sender) && to != payable(address(this))) revert(); to.transfer(amount); }
    function asymmetricDestinationDisjunctionBad(address payable to, uint256 amount) external {
        require(to == payable(msg.sender) || amount > 0);
        to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract LiteralAndSelfRestrictedCaller {
    modifier onlySelf() { require(msg.sender == address(this)); _; }
    modifier onlySelf2() { require(msg.sender == payable(address(this))); _; }
    modifier onlyHardcodedAdmin() { require(msg.sender == 0x1234567890123456789012345678901234567890); _; }
    modifier onlyHardcodedAdminCast() { require(msg.sender == payable(0x1234567890123456789012345678901234567890)); _; }
    modifier onlyZeroAddress() { require(msg.sender == address(0)); _; }
    function selfRestrictedBad(address payable to, uint256 amount) external onlySelf { to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function self2RestrictedBad(address payable to, uint256 amount) external onlySelf2 { to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function hardcodedAdminOk(address payable to, uint256 amount) external onlyHardcodedAdmin { to.transfer(amount); }
    function hardcodedAdminCastOk(address payable to, uint256 amount) external onlyHardcodedAdminCast { to.transfer(amount); }
    function zeroAddressGuardOk(address payable to, uint256 amount) external onlyZeroAddress { to.transfer(amount); }
}

// Self-alias trampolines: planting `address(this)` into any state slot through
// any laundering path (inline init, immutable, chain, struct, mapping, tuple,
// 5-hop chain, ctor helper, struct literal, ctor local, nested helper, ternary,
// array push) must reject the slot as a trusted caller principal.
// (`address(this)` itself as a guard principal is exercised via `selfRestrictedBad`
// in LiteralAndSelfRestrictedCaller below.)
contract SelfAliasTrampolineMega {
    struct Cfg { address self; }
    address public SELF_INLINE = address(this);
    address public immutable SELF_IMMUTABLE;
    address public SELF_CHAIN_SRC = address(this);
    address public SELF_CHAIN = SELF_CHAIN_SRC;
    Cfg cfg;
    mapping(uint256 => address) principals;
    address tupleSelf;
    address tupleOther;
    address dA; address dB; address dC; address dD = address(this);
    address fA; address fB; address fC; address fD; address fE;
    address SELF_HELPER;
    Cfg cfgLit;
    address SELF_LOCAL;
    address SELF_NESTED;
    bool tFlag;
    address tSeed;
    address SELF_TERNARY;
    address[] pushArr;
    function _initHelper() internal { SELF_HELPER = address(this); }
    function _initOuter() internal { _initInner(); }
    function _initInner() internal { SELF_NESTED = address(this); }
    constructor(address other) {
        SELF_IMMUTABLE = address(this);
        cfg.self = address(this);
        principals[0] = address(this);
        (tupleSelf, tupleOther) = (address(this), other);
        dC = dD; dB = dC; dA = dB;
        fE = address(this); fD = fE; fC = fD; fB = fC; fA = fB;
        _initHelper();
        cfgLit = Cfg(address(this));
        address local = address(this); SELF_LOCAL = local;
        _initOuter();
        tSeed = other; SELF_TERNARY = tFlag ? address(this) : tSeed;
        pushArr.push(address(this));
    }
    function dInline(address payable to, uint256 a) external { require(msg.sender == SELF_INLINE); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dImmutable(address payable to, uint256 a) external { require(msg.sender == SELF_IMMUTABLE); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dChain(address payable to, uint256 a) external { require(msg.sender == SELF_CHAIN); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dStruct(address payable to, uint256 a) external { require(msg.sender == cfg.self); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dMapping(address payable to, uint256 a) external { require(msg.sender == principals[0]); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dTuple(address payable to, uint256 a) external { require(msg.sender == tupleSelf); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dDeep(address payable to, uint256 a) external { require(msg.sender == dA); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dFiveHop(address payable to, uint256 a) external { require(msg.sender == fA); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dCtorHelper(address payable to, uint256 a) external { require(msg.sender == SELF_HELPER); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dStructLit(address payable to, uint256 a) external { require(msg.sender == cfgLit.self); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dCtorLocal(address payable to, uint256 a) external { require(msg.sender == SELF_LOCAL); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dNestedHelper(address payable to, uint256 a) external { require(msg.sender == SELF_NESTED); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dTernary(address payable to, uint256 a) external { require(msg.sender == SELF_TERNARY); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dPush(address payable to, uint256 a) external { require(msg.sender == pushArr[0]); to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

// Literal/zero principals + nested-boolean destination guards.
contract GuardShapesAndLiterals {
    function bareZeroOk(address payable to, uint256 amt) external { require(msg.sender == address(0)); to.transfer(amt); }
    function bareAddressLiteralOk(address payable to, uint256 amt) external { require(msg.sender == 0x0000000000000000000000000000000000000001); to.transfer(amt); }
    function deMorganNestedOk(address payable to, uint256 amt) external { require(!(to != payable(msg.sender) && to != payable(address(this)))); to.transfer(amt); }
    function conjThenDisjOk(address payable to, uint256 amt) external { require(amt > 0 && (to == payable(msg.sender) || to == payable(address(this)))); to.transfer(amt); }
    function threeWayDisjOk(address payable to, uint256 amt) external {
        address payable self = payable(address(this));
        address payable sender = payable(msg.sender);
        require(to == self || to == sender || to == payable(0x000000000000000000000000000000000000dEaD));
        to.transfer(amt);
    }
    function threeWayAsymmetricBad(address payable to, uint256 amt) external {
        require(to == payable(msg.sender) || to == payable(address(this)) || amt > 0);
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract FunctionPointerSinks {
    function() external payable[] public callbacks;
    mapping(bytes4 => function() external payable) public handlers;
    function pushCallback(function() external payable cb) external { callbacks.push(cb); }
    function fireBad(uint256 i) external payable { callbacks[i]{value: msg.value}(); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function fireMappingBad(bytes4 sel) external payable { handlers[sel]{value: msg.value}(); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function zeroValueFireOk(uint256 i) external { callbacks[i]{value: 0}(); }
}

contract InlineAssemblyClobber {
    function asmClobberBad(address payable other, uint256 amount) external {
        address payable to = payable(msg.sender);
        assembly {
            to := other
        }
        to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function asmAfterSinkOk(address payable other, uint256 amount) external {
        address payable to = payable(msg.sender);
        to.transfer(amount);
        assembly {
            to := other
        }
    }
}

contract DocumentedLimitations {
    address public owner;
    constructor(address _o) { owner = _o; }
    modifier onlyOwnerMutable() { require(msg.sender == owner); _; }
    function setOwnerUnprotected(address newOwner) external { owner = newOwner; }
    function mutableOwnerSuppressesBad(address payable to, uint256 amount) external onlyOwnerMutable { to.transfer(amount); }
    function externalProtected(address payable to, uint256 amount) external onlyOwnerMutable { _send(to, amount); }
    function _send(address payable to, uint256 amount) internal { to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

interface ITryProbe {
    function ping() external;
}

// Try/catch fact survival, all-clauses-exiting, isolation, nested-block-revert.
contract TryClauseTests {
    address public immutable owner;
    constructor(address _o) { owner = _o; }
    function callerFactInBothClausesOk(ITryProbe p, address payable to, uint256 amt) external {
        try p.ping() { require(msg.sender == owner); } catch { require(msg.sender == owner); }
        to.transfer(amt);
    }
    function destFactInBothClausesOk(ITryProbe p, address payable to, uint256 amt) external {
        try p.ping() { require(to == payable(msg.sender)); } catch { require(to == payable(msg.sender)); }
        to.transfer(amt);
    }
    function asymmetricTryClauseBad(ITryProbe p, address payable to, uint256 amt) external {
        try p.ping() { require(msg.sender == owner); } catch {}
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function allClausesExitOk(ITryProbe p, address payable to, uint256 amt) external {
        try p.ping() { revert(); } catch { revert(); }
        to.transfer(amt); // unreachable
    }
    function tryClauseIsolationOk(
        IBaseRecipient probe, address payable to, address payable other, uint256 amt
    ) external {
        require(to == payable(msg.sender));
        try probe.recipient() returns (address payable) { to = other; }
        catch { to.transfer(amt); }
    }
    function unreachableAfterNestedBlockExitOk(address payable to, uint256 amt) external {
        { revert(); }
        to.transfer(amt);
    }
}

// Selfdestruct-as-exit + abi.decode receivers merged.
contract ExitsAndAbiDecode {
    function selfdestructThenSinkOk(address payable to, uint256 amt) external {
        selfdestruct(payable(address(this)));
        to.transfer(amt); // unreachable
    }
    function selfdestructInBothBranchesOk(address payable to, uint256 amt, bool b) external {
        if (b) { selfdestruct(payable(address(this))); } else { revert(); }
        to.transfer(amt); // both branches exit
    }
    function abiDecodePayableTransferBad(bytes calldata data, uint256 amt) external { payable(abi.decode(data, (address))).transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function abiDecodeCallWithValueBad(bytes calldata data, uint256 amt) external returns (bool ok) { (ok,) = abi.decode(data, (address)).call{value: amt}(""); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

// Runtime-planted self alias + named-arg ctor helper self alias.
contract SelfAliasRuntimeAndNamedBad {
    address selfRuntime;
    address selfNamed;
    constructor() {
        _init({ignored: 1, self_: address(this)});
    }
    function _init(uint256 ignored, address self_) internal { ignored; selfNamed = self_; }
    function plantSelfAlias() external { selfRuntime = address(this); }
    function dRuntime(address payable to, uint256 a) external {
        require(msg.sender == selfRuntime);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function dNamed(address payable to, uint256 a) external {
        require(msg.sender == selfNamed);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

// Wrapper-nested vs conditional placeholder. Nested-in-block is caller-restricting.
// Nested under `if`: post-guard cannot retroactively restrict body.
contract PlaceholderShapes {
    address public immutable owner;
    constructor(address _o) { owner = _o; }
    modifier onlyOwnerNested() { { require(msg.sender == owner); _; } }
    modifier maybePlaceholderThenGuard(bool b) {
        if (b) { _; }
        require(msg.sender == owner);
    }
    function withdrawOk(address payable to, uint256 amt) external onlyOwnerNested { to.transfer(amt); }
    function notRestrictedBad(address payable to, uint256 amt, bool b) external maybePlaceholderThenGuard(b) { to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract WrapperPrefixParamReassignmentBad {
    modifier checkOuterAssign(address payable who) {
        who = payable(msg.sender);
        {
            require(who == payable(msg.sender));
            _;
        }
    }
    function paramReassignedThenWrapperBad(address payable to, uint256 amount) external checkOuterAssign(to) { to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

// Ternary side-effect + assignment receiver — small regressions.
contract TernaryAndAssignBad {
    function ternarySideEffectBad(bool flag, address payable to, uint256 amt) external {
        flag ? true : ((to = payable(msg.sender)) == payable(msg.sender));
        to.transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function assignmentReceiverBad(address payable to, address payable other, uint256 amt) external { (to = other).transfer(amt); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

library IdLib {
    function id(address a) internal pure returns (address) { return a; }
}

// Merged: 7 self-alias laundering paths into a single contract. Each `pay*`
// exercises one bypass shape; all are expected to warn.
contract SelfAliasLaunderingBad {
    struct Cfg { address self; }
    address public aNumericCast = address(uint160(address(this)));
    address public aIdentity;
    address public aIdentityLib;
    address public aIdentityNamed;
    address public aIdentityCast;
    address public aAggregateCopy;
    address public aLibraryNoArg;
    Cfg public cfg;
    address public aModifierArg;
    function id(address x) internal pure returns (address) { return x; }
    function idNamed(uint256 ignored, address a) internal pure returns (address) { return a; }
    function idCast(address a) internal pure returns (address) { return address(uint160(a)); }
    modifier plant(address a) { aModifierArg = a; _; }
    constructor() {
        aIdentity = id(address(this));
        aIdentityLib = IdLib.id(address(this));
        aIdentityNamed = idNamed({a: address(this), ignored: 0});
        aIdentityCast = idCast(address(this));
        cfg.self = address(this);
        aAggregateCopy = cfg.self;
        aLibraryNoArg = SelfRetLib.self();
    }
    function init() external plant(address(this)) {}
    function payNumeric(address payable to, uint256 a) external {
        require(msg.sender == aNumericCast);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payIdentity(address payable to, uint256 a) external {
        require(msg.sender == aIdentity);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payIdentityLib(address payable to, uint256 a) external {
        require(msg.sender == aIdentityLib);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payIdentityNamed(address payable to, uint256 a) external {
        require(msg.sender == aIdentityNamed);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payIdentityCast(address payable to, uint256 a) external {
        require(msg.sender == aIdentityCast);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payAggregateCopy(address payable to, uint256 a) external {
        require(msg.sender == aAggregateCopy);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payLibraryNoArg(address payable to, uint256 a) external {
        require(msg.sender == aLibraryNoArg);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
    function payModifierArg(address payable to, uint256 a) external {
        require(msg.sender == aModifierArg);
        to.transfer(a); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract InheritedTrustedBase {
    address internal inheritedTrusted;
}

contract InheritedTrustedDerivedBad is InheritedTrustedBase {
    constructor() { inheritedTrusted = address(this); }
    function pay(address payable to, uint256 amount) external {
        require(msg.sender == inheritedTrusted);
        to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract BaseSelfArg {
    address internal baseSelf;
    constructor(address a) { baseSelf = a; }
}

contract DerivedBaseCtorSelfAliasBad is BaseSelfArg {
    constructor() BaseSelfArg(address(this)) {}
    function pay(address payable to, uint256 amount) external {
        require(msg.sender == baseSelf);
        to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract Pusher {
    function push(address) external {}
}

contract PushNameCollisionOk {
    Pusher public trusted;
    constructor(Pusher p) { trusted = p; }
    function touch() external { trusted.push(address(this)); }
    function pay(address payable to, uint256 amount) external {
        require(msg.sender == address(trusted));
        to.transfer(amount); // must NOT warn — `push` here is unrelated to builtin array push
    }
}

library OverId {
    function id(address a) internal pure returns (address) { return a; }
    function id(address, uint256) internal pure returns (address) { return 0x000000000000000000000000000000000000dEaD; }
}

contract OverloadedStaticIdentityArityOk {
    address public trusted;
    constructor() { trusted = OverId.id(address(this), 1); }
    function pay(address payable to, uint256 amount) external {
        require(msg.sender == trusted);
        to.transfer(amount); // must NOT warn — wrong-arity overload is not identity
    }
}

contract BaseSelfChain {
    address internal chainSelf;
    constructor(address a) { chainSelf = a; }
}

contract MidSelfChain is BaseSelfChain {
    constructor(address a) BaseSelfChain(a) {}
}

contract LeafSelfChainBad is MidSelfChain {
    constructor() MidSelfChain(address(this)) {}
    function pay(address payable to, uint256 amount) external {
        require(msg.sender == chainSelf);
        to.transfer(amount); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

library SelfRetLib {
    function self() internal view returns (address) { return address(this); }
}

contract NumericCastSafeOk {
    function payZeroViaUint160Ok(uint256 amt) external { payable(address(uint160(0))).transfer(amt); }
}

contract FnPtrFromSelf {
    receive() external payable {}
    function receiveEth() external payable {}
    function selfPtrOk(uint256 amt) external payable {
        function() external payable cb = this.receiveEth;
        cb{value: amt}();
    }
    function paramPtrBad(function() external payable cb, uint256 amt) external payable {
        cb{value: amt}(); //~WARN: ETH is sent to a user-controlled destination; restrict the destination or the caller
    }
}

contract TrailingReturnHelperOk {
    address public immutable owner;
    constructor() { owner = msg.sender; }
    function _checkOwner() internal view { if (msg.sender != owner) revert(); return; }
    modifier onlyOwner() { _checkOwner(); _; }
    function withdraw(address payable to, uint256 amt) external onlyOwner { to.transfer(amt); }
}

abstract contract OwnableStateGuard {
    address private _owner;

    function _msgSender() internal view returns (address) { return msg.sender; }
    function owner() public view returns (address) { return _owner; }
    function _checkOwner() internal view { require(owner() == _msgSender(), "not owner"); }
    modifier onlyOwner() { _checkOwner(); _; }
    function _transferOwnership(address newOwner) internal {
        address oldOwner = _owner;
        _owner = newOwner;
        oldOwner;
    }
}

contract OwnableModifierNoSinkRegression is OwnableStateGuard {
    uint256 public value;

    function init(address newOwner) external { _transferOwnership(newOwner); }
    function set0(uint256 x) external onlyOwner { value = x; }
    function set1(uint256 x) external onlyOwner { value = x; }
    function set2(uint256 x) external onlyOwner { value = x; }
    function set3(uint256 x) external onlyOwner { value = x; }
    function set4(uint256 x) external onlyOwner { value = x; }
    function set5(uint256 x) external onlyOwner { value = x; }
    function set6(uint256 x) external onlyOwner { value = x; }
    function set7(uint256 x) external onlyOwner { value = x; }
    function restrictedPay(address payable to, uint256 amount) external onlyOwner {
        to.transfer(amount);
    }
}
