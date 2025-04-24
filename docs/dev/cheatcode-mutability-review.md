# Cheatcode Mutability Review Checklist

This document tracks all cheatcodes currently marked `external` with no `view` or `pure` mutability modifier.

Total reviewed: **231**

---

## Summary

| Mutability | Count | Description |
|------------|-------|-------------|
| `pure`     | 1     | Stateless helper functions |
| `view`     | 11    | Reads test environment state |
| `-` (keep as is) | 219 | Modifies test state or relies on test state |

---

## Review Criteria

From issue [#10027](https://github.com/foundry-rs/foundry/issues/10027):

- If a cheatcode **modifies observable state** for later cheatcode calls (EVM, interpreter, filesystem), it is neither `view` nor `pure`
- If a cheatcode **reads** from prior cheatcode state or environment state, it is `view`
- If a cheatcode **has no side effects** and doesn’t depend on test stat, it is `pure`

---

## Mutability Suggestions

### Pure Cheatcodes

Mark `pure`:

- [x] `contains`: `contains(string,string)` _suggested: `pure`_

---

### View Cheatcodes

Mark `view`:

- [x] `accessList`: `accessList(address,bytes32[])` _resolved: not view, keep as is due to test coverage indicating stateful behavior_
- [ ] `accesses`: `accesses(address)`
- [ ] `eth_getLogs`: `eth_getLogs(uint256,uint256,address,bytes32[])`
- [ ] `getMappingKeyAndParentOf`: `getMappingKeyAndParentOf(address,bytes32)`
- [ ] `getMappingLength`: `getMappingLength(address,bytes32)`
- [ ] `getMappingSlotAt`: `getMappingSlotAt(address,bytes32,uint256)`
- [ ] `getNonce_1`: `getNonce(address,uint256,uint256,uint256)`
- [ ] `getRecordedLogs`: `getRecordedLogs()`
- [ ] `getWallets`: `getWallets()`
- [ ] `noAccessList`: `noAccessList()`
- [ ] `readCallers`: `readCallers()`

---

### Keep As Is

Not `view` nor `pure`:

- [ ] `_expectCheatcodeRevert_0`: `_expectCheatcodeRevert()` _suggested: keep as is_
- [ ] `_expectCheatcodeRevert_1`: `_expectCheatcodeRevert(bytes4)` _suggested: keep as is_
- [ ] `_expectCheatcodeRevert_2`: `_expectCheatcodeRevert(bytes)` _suggested: keep as is_
- [ ] `allowCheatcodes`: `allowCheatcodes(address)` _suggested: keep as is_
- [ ] `attachBlob`: `attachBlob(bytes)` _suggested: keep as is_
- [ ] `attachDelegation`: `attachDelegation((uint8,bytes32,bytes32,uint64,address))` _suggested: keep as is_
- [ ] `blobBaseFee`: `blobBaseFee(uint256)` _suggested: keep as is_
- [ ] `blobhashes`: `blobhashes(bytes32[])` _suggested: keep as is_
- [ ] `broadcastRawTransaction`: `broadcastRawTransaction(bytes)` _suggested: keep as is_
- [ ] `broadcast_0`: `broadcast()` _suggested: keep as is_
- [ ] `broadcast_1`: `broadcast(address)` _suggested: keep as is_
- [ ] `broadcast_2`: `broadcast(uint256)` _suggested: keep as is_
- [ ] `chainId`: `chainId(uint256)` _suggested: keep as is_
- [ ] `clearMockedCalls`: `clearMockedCalls()` _suggested: keep as is_
- [ ] `cloneAccount`: `cloneAccount(address,address)` _suggested: keep as is_
- [ ] `closeFile`: `closeFile(string)` _suggested: keep as is_
- [ ] `coinbase`: `coinbase(address)` _suggested: keep as is_
- [ ] `cool`: `cool(address)` _suggested: keep as is_
- [ ] `coolSlot`: `coolSlot(address,bytes32)` _suggested: keep as is_
- [ ] `copyFile`: `copyFile(string,string)` _suggested: keep as is_
- [ ] `copyStorage`: `copyStorage(address,address)` _suggested: keep as is_
- [ ] `createDir`: `createDir(string,bool)` _suggested: keep as is_
- [ ] `createFork_0`: `createFork(string)` _suggested: keep as is_
- [ ] `createFork_1`: `createFork(string,uint256)` _suggested: keep as is_
- [ ] `createFork_2`: `createFork(string,bytes32)` _suggested: keep as is_
- [ ] `createSelectFork_0`: `createSelectFork(string)` _suggested: keep as is_
- [ ] `createSelectFork_1`: `createSelectFork(string,uint256)` _suggested: keep as is_
- [ ] `createSelectFork_2`: `createSelectFork(string,bytes32)` _suggested: keep as is_
- [ ] `createWallet_0`: `createWallet(string)` _suggested: keep as is_
- [ ] `createWallet_1`: `createWallet(uint256)` _suggested: keep as is_
- [ ] `createWallet_2`: `createWallet(uint256,string)` _suggested: keep as is_
- [ ] `deal`: `deal(address,uint256)` _suggested: keep as is_
- [ ] `deleteSnapshot`: `deleteSnapshot(uint256)` _suggested: keep as is_
- [ ] `deleteSnapshots`: `deleteSnapshots()` _suggested: keep as is_
- [ ] `deleteStateSnapshot`: `deleteStateSnapshot(uint256)` _suggested: keep as is_
- [ ] `deleteStateSnapshots`: `deleteStateSnapshots()` _suggested: keep as is_
- [ ] `deployCode_0`: `deployCode(string)` _suggested: keep as is_
- [ ] `deployCode_1`: `deployCode(string,bytes)` _suggested: keep as is_
- [ ] `deployCode_2`: `deployCode(string,uint256)` _suggested: keep as is_
- [ ] `deployCode_3`: `deployCode(string,bytes,uint256)` _suggested: keep as is_
- [ ] `deployCode_4`: `deployCode(string,bytes32)` _suggested: keep as is_
- [ ] `deployCode_5`: `deployCode(string,bytes,bytes32)` _suggested: keep as is_
- [ ] `deployCode_6`: `deployCode(string,uint256,bytes32)` _suggested: keep as is_
- [ ] `deployCode_7`: `deployCode(string,bytes,uint256,bytes32)` _suggested: keep as is_
- [ ] `difficulty`: `difficulty(uint256)` _suggested: keep as is_
- [ ] `dumpState`: `dumpState(string)` _suggested: keep as is_
- [ ] `etch`: `etch(address,bytes)` _suggested: keep as is_
- [ ] `expectCallMinGas_0`: `expectCallMinGas(address,uint256,uint64,bytes)` _suggested: keep as is_
- [ ] `expectCallMinGas_1`: `expectCallMinGas(address,uint256,uint64,bytes,uint64)` _suggested: keep as is_
- [ ] `expectCall_0`: `expectCall(address,bytes)` _suggested: keep as is_
- [ ] `expectCall_1`: `expectCall(address,bytes,uint64)` _suggested: keep as is_
- [ ] `expectCall_2`: `expectCall(address,uint256,bytes)` _suggested: keep as is_
- [ ] `expectCall_3`: `expectCall(address,uint256,bytes,uint64)` _suggested: keep as is_
- [ ] `expectCall_4`: `expectCall(address,uint256,uint64,bytes)` _suggested: keep as is_
- [ ] `expectCall_5`: `expectCall(address,uint256,uint64,bytes,uint64)` _suggested: keep as is_
- [ ] `expectCreate`: `expectCreate(bytes,address)`_suggested: keep as is_
- [ ] `expectCreate2`: `expectCreate2(bytes,address)` _suggested: keep as is_
- [ ] `expectEmitAnonymous_0`: `expectEmitAnonymous(bool,bool,bool,bool,bool)` _suggested: keep as is_
- [ ] `expectEmitAnonymous_1`: `expectEmitAnonymous(bool,bool,bool,bool,bool,address)` _suggested: keep as is_
- [ ] `expectEmitAnonymous_2`: `expectEmitAnonymous()` _suggested: keep as is_
- [ ] `expectEmitAnonymous_3`: `expectEmitAnonymous(address)` _suggested: keep as is_
- [ ] `expectEmit_0`: `expectEmit(bool,bool,bool,bool)` _suggested: keep as is_
- [ ] `expectEmit_1`: `expectEmit(bool,bool,bool,bool,address)` _suggested: keep as is_
- [ ] `expectEmit_2`: `expectEmit()` _suggested: keep as is_
- [ ] `expectEmit_3`: `expectEmit(address)` _suggested: keep as is_
- [ ] `expectEmit_4`: `expectEmit(bool,bool,bool,bool,uint64)` _suggested: keep as is_
- [ ] `expectEmit_5`: `expectEmit(bool,bool,bool,bool,address,uint64)` _suggested: keep as is_
- [ ] `expectEmit_6`: `expectEmit(uint64)` _suggested: keep as is_
- [ ] `expectEmit_7`: `expectEmit(address,uint64)` _suggested: keep as is_
- [ ] `expectPartialRevert_0`: `expectPartialRevert(bytes4)`  _suggested: keep as is_
- [ ] `expectPartialRevert_1`: `expectPartialRevert(bytes4,address)`  _suggested: keep as is_
- [ ] `expectRevert_0`: `expectRevert()`  _suggested: keep as is_
- [ ] `expectRevert_1`: `expectRevert(bytes4)`  _suggested: keep as is_
- [ ] `expectRevert_10`: `expectRevert(bytes4,address,uint64)`  _suggested: keep as is_
- [ ] `expectRevert_11`: `expectRevert(bytes,address,uint64)`  _suggested: keep as is_
- [ ] `expectRevert_2`: `expectRevert(bytes)`  _suggested: keep as is_
- [ ] `expectRevert_3`: `expectRevert(address)`  _suggested: keep as is_
- [ ] `expectRevert_4`: `expectRevert(bytes4,address)`  _suggested: keep as is_
- [ ] `expectRevert_5`: `expectRevert(bytes,address)`  _suggested: keep as is_
- [ ] `expectRevert_6`: `expectRevert(uint64)`  _suggested: keep as is_
- [ ] `expectRevert_7`: `expectRevert(bytes4,uint64)`  _suggested: keep as is_
- [ ] `expectRevert_8`: `expectRevert(bytes,uint64)`  _suggested: keep as is_
- [ ] `expectRevert_9`: `expectRevert(address,uint64)`  _suggested: keep as is_
- [ ] `expectSafeMemory`: `expectSafeMemory(uint64,uint64)`  _suggested: keep as is_
- [ ] `expectSafeMemoryCall`: `expectSafeMemoryCall(uint64,uint64)`  _suggested: keep as is_
- [ ] `fee`: `fee(uint256)` _suggested: keep as is_
- [ ] `ffi`: `ffi(string[])` _suggested: keep as is_
- [ ] `interceptInitcode`: `interceptInitcode()` _suggested: keep as is_
- [ ] `label`: `label(address,string)` _suggested: keep as is_
- [ ] `loadAllocs`: `loadAllocs(string)` _suggested: keep as is_
- [ ] `makePersistent_0`: `makePersistent(address)` _suggested: keep as is_
- [ ] `makePersistent_1`: `makePersistent(address,address)` _suggested: keep as is_
- [ ] `makePersistent_2`: `makePersistent(address,address,address)` _suggested: keep as is_
- [ ] `makePersistent_3`: `makePersistent(address[])` _suggested: keep as is_
- [ ] `mockCallRevert_0`: `mockCallRevert(address,bytes,bytes)` _suggested: keep as is_
- [ ] `mockCallRevert_1`: `mockCallRevert(address,uint256,bytes,bytes)` _suggested: keep as is_
- [ ] `mockCallRevert_2`: `mockCallRevert(address,bytes4,bytes)` _suggested: keep as is_
- [ ] `mockCallRevert_3`: `mockCallRevert(address,uint256,bytes4,bytes)` _suggested: keep as is_
- [ ] `mockCall_0`: `mockCall(address,bytes,bytes)` _suggested: keep as is_
- [ ] `mockCall_1`: `mockCall(address,uint256,bytes,bytes)` _suggested: keep as is_
- [ ] `mockCall_2`: `mockCall(address,bytes4,bytes)` _suggested: keep as is_
- [ ] `mockCall_3`: `mockCall(address,uint256,bytes4,bytes)` _suggested: keep as is_
- [ ] `mockCalls_0`: `mockCalls(address,bytes,bytes[])` _suggested: keep as is_
- [ ] `mockCalls_1`: `mockCalls(address,uint256,bytes,bytes[])` _suggested: keep as is_
- [ ] `mockFunction`: `mockFunction(address,address,bytes)` _suggested: keep as is_
- [ ] `pauseGasMetering`: `pauseGasMetering()` _suggested: keep as is_
- [ ] `prank_0`: `prank(address)` _suggested: keep as is_
- [ ] `prank_1`: `prank(address,address)` _suggested: keep as is_
- [ ] `prank_2`: `prank(address,bool)` _suggested: keep as is_
- [ ] `prank_3`: `prank(address,address,bool)` _suggested: keep as is_
- [ ] `prevrandao_0`: `prevrandao(bytes32)` _suggested: keep as is_
- [ ] `prevrandao_1`: `prevrandao(uint256)` _suggested: keep as is_
- [ ] `prompt`: `prompt(string)` _suggested: keep as is_
- [ ] `promptAddress`: `promptAddress(string)` _suggested: keep as is_
- [ ] `promptSecret`: `promptSecret(string)` _suggested: keep as is_
- [ ] `promptSecretUint`: `promptSecretUint(string)` _suggested: keep as is_
- [ ] `promptUint`: `promptUint(string)` _suggested: keep as is_
- [ ] `randomAddress`: `randomAddress()` _suggested: keep as is_
- [ ] `randomUint_0`: `randomUint()` _suggested: keep as is_
- [ ] `randomUint_1`: `randomUint(uint256,uint256)` _suggested: keep as is_
- [ ] `record`: `record()` _suggested: keep as is_
- [ ] `recordLogs`: `recordLogs()` _suggested: keep as is_
- [ ] `rememberKey`: `rememberKey(uint256)` _suggested: keep as is_
- [ ] `rememberKeys_0`: `rememberKeys(string,string,uint32)` _suggested: keep as is_
- [ ] `rememberKeys_1`: `rememberKeys(string,string,string,uint32)` _suggested: keep as is_
- [ ] `removeDir`: `removeDir(string,bool)`  _suggested: keep as is_
- [ ] `removeFile`: `removeFile(string)`  _suggested: keep as is_
- [ ] `resetGasMetering`: `resetGasMetering()`  _suggested: keep as is_
- [ ] `resetNonce`: `resetNonce(address)`  _suggested: keep as is_
- [ ] `resumeGasMetering`: `resumeGasMetering()`  _suggested: keep as is_
- [ ] `revertTo`: `revertTo(uint256)`  _suggested: keep as is_
- [ ] `revertToAndDelete`: `revertToAndDelete(uint256)`  _suggested: keep as is_
- [ ] `revertToState`: `revertToState(uint256)`  _suggested: keep as is_
- [ ] `revertToStateAndDelete`: `revertToStateAndDelete(uint256)`  _suggested: keep as is_
- [ ] `revokePersistent_0`: `revokePersistent(address)` _suggested: keep as is_
- [ ] `revokePersistent_1`: `revokePersistent(address[])` _suggested: keep as is_
- [ ] `roll`: `roll(uint256)` _suggested: keep as is_
- [ ] `rollFork_0`: `rollFork(uint256)` _suggested: keep as is_
- [ ] `rollFork_1`: `rollFork(bytes32)` _suggested: keep as is_
- [ ] `rollFork_2`: `rollFork(uint256,uint256)` _suggested: keep as is_
- [ ] `rollFork_3`: `rollFork(uint256,bytes32)` _suggested: keep as is_
- [ ] `rpc_0`: `rpc(string,string)`  _suggested: keep as is_
- [ ] `rpc_1`: `rpc(string,string,string)`  _suggested: keep as is_
- [ ] `selectFork`: `selectFork(uint256)` _suggested: keep as is_
- [ ] `serializeAddress_0`: `serializeAddress(string,string,address)` _suggested: keep as is_
- [ ] `serializeAddress_1`: `serializeAddress(string,string,address[])` _suggested: keep as is_
- [ ] `serializeBool_0`: `serializeBool(string,string,bool)` _suggested: keep as is_
- [ ] `serializeBool_1`: `serializeBool(string,string,bool[])` _suggested: keep as is_
- [ ] `serializeBytes32_0`: `serializeBytes32(string,string,bytes32)` _suggested: keep as is_
- [ ] `serializeBytes32_1`: `serializeBytes32(string,string,bytes32[])` _suggested: keep as is_
- [ ] `serializeBytes_0`: `serializeBytes(string,string,bytes)` _suggested: keep as is_
- [ ] `serializeBytes_1`: `serializeBytes(string,string,bytes[])` _suggested: keep as is_
- [ ] `serializeInt_0`: `serializeInt(string,string,int256)` _suggested: keep as is_
- [ ] `serializeInt_1`: `serializeInt(string,string,int256[])` _suggested: keep as is_
- [ ] `serializeJson`: `serializeJson(string,string)` _suggested: keep as is_
- [ ] `serializeJsonType_1`: `serializeJsonType(string,string,string,bytes)` _suggested: keep as is_
- [ ] `serializeString_0`: `serializeString(string,string,string)` _suggested: keep as is_
- [ ] `serializeString_1`: `serializeString(string,string,string[])` _suggested: keep as is_
- [ ] `serializeUintToHex`: `serializeUintToHex(string,string,uint256)` _suggested: keep as is_
- [ ] `serializeUint_0`: `serializeUint(string,string,uint256)` _suggested: keep as is_
- [ ] `serializeUint_1`: `serializeUint(string,string,uint256[])` _suggested: keep as is_
- [ ] `setArbitraryStorage_0`: `setArbitraryStorage(address)` _suggested: keep as is_
- [ ] `setArbitraryStorage_1`: `setArbitraryStorage(address,bool)` _suggested: keep as is_
- [ ] `setBlockhash`: `setBlockhash(uint256,bytes32)` _suggested: keep as is_
- [ ] `setEnv`: `setEnv(string,string)` _suggested: keep as is_
- [ ] `setNonce`: `setNonce(address,uint64)` _suggested: keep as is_
- [ ] `setNonceUnsafe`: `setNonceUnsafe(address,uint64)` _suggested: keep as is_
- [ ] `shuffle`: `shuffle(uint256[])` _suggested: keep as is_
- [ ] `signAndAttachDelegation_0`: `signAndAttachDelegation(address,uint256)` _suggested: keep as is_
- [ ] `signAndAttachDelegation_1`: `signAndAttachDelegation(address,uint256,uint64)` _suggested: keep as is_
- [ ] `signCompact_0`: `signCompact((address,uint256,uint256,uint256),bytes32)` _suggested: keep as is_
- [ ] `signDelegation_0`: `signDelegation(address,uint256)` _suggested: keep as is_
- [ ] `signDelegation_1`: `signDelegation(address,uint256,uint64)` _suggested: keep as is_
- [ ] `sign_0`: `sign((address,uint256,uint256,uint256),bytes32)` _suggested: keep as is_
- [ ] `skip_0`: `skip(bool)` _suggested: keep as is_
- [ ] `skip_1`: `skip(bool,string)` _suggested: keep as is_
- [ ] `sleep`: `sleep(uint256)`  _suggested: keep as is_
- [ ] `snapshot`: `snapshot()` _suggested: keep as is_
- [ ] `snapshotGasLastCall_0`: `snapshotGasLastCall(string)`  _suggested: keep as is_
- [ ] `snapshotGasLastCall_1`: `snapshotGasLastCall(string,string)`  _suggested: keep as is_
- [ ] `snapshotState`: `snapshotState()` _suggested: keep as is_
- [ ] `snapshotValue_0`: `snapshotValue(string,uint256)` _suggested: keep as is_
- [ ] `snapshotValue_1`: `snapshotValue(string,string,uint256)` _suggested: keep as is_
- [ ] `sort`: `sort(uint256[])` _suggested: keep as is_
- [ ] `startBroadcast_0`: `startBroadcast()` _suggested: keep as is_
- [ ] `startBroadcast_1`: `startBroadcast(address)` _suggested: keep as is_
- [ ] `startBroadcast_2`: `startBroadcast(uint256)` _suggested: keep as is_
- [ ] `startDebugTraceRecording`: `startDebugTraceRecording()` _suggested: keep as is_
- [ ] `startMappingRecording`: `startMappingRecording()` _suggested: keep as is_
- [ ] `startPrank_0`: `startPrank(address)` _suggested: keep as is_
- [ ] `startPrank_1`: `startPrank(address,address)` _suggested: keep as is_
- [ ] `startPrank_2`: `startPrank(address,bool)` _suggested: keep as is_
- [ ] `startPrank_3`: `startPrank(address,address,bool)` _suggested: keep as is_
- [ ] `startSnapshotGas_0`: `startSnapshotGas(string)` _suggested: keep as is_
- [ ] `startSnapshotGas_1`: `startSnapshotGas(string,string)` _suggested: keep as is_
- [ ] `startStateDiffRecording`: `startStateDiffRecording()` _suggested: keep as is_
- [ ] `stopAndReturnDebugTraceRecording`: `stopAndReturnDebugTraceRecording()` _suggested: keep as is_
- [ ] `stopAndReturnStateDiff`: `stopAndReturnStateDiff()` _suggested: keep as is_
- [ ] `stopBroadcast`: `stopBroadcast()` _suggested: keep as is_
- [ ] `stopExpectSafeMemory`: `stopExpectSafeMemory()` _suggested: keep as is_
- [ ] `stopMappingRecording`: `stopMappingRecording()` _suggested: keep as is_
- [ ] `stopPrank`: `stopPrank()` _suggested: keep as is_
- [ ] `stopSnapshotGas_0`: `stopSnapshotGas()` _suggested: keep as is_
- [ ] `stopSnapshotGas_1`: `stopSnapshotGas(string)` _suggested: keep as is_
- [ ] `stopSnapshotGas_2`: `stopSnapshotGas(string,string)` _suggested: keep as is_
- [ ] `store`: `store(address,bytes32,bytes32)` _suggested: keep as is_
- [ ] `transact_0`: `transact(bytes32)` _suggested: keep as is_
- [ ] `transact_1`: `transact(uint256,bytes32)` _suggested: keep as is_
- [ ] `tryFfi`: `tryFfi(string[])` _suggested: keep as is_
- [ ] `txGasPrice`: `txGasPrice(uint256)` _suggested: keep as is_
- [ ] `warmSlot`: `warmSlot(address,bytes32)`  _suggested: keep as is_
- [ ] `warp`: `warp(uint256)` _suggested: keep as is_
- [ ] `writeFile`: `writeFile(string,string)` _suggested: keep as is_
- [ ] `writeFileBinary`: `writeFileBinary(string,bytes)` _suggested: keep as is_
- [ ] `writeJson_0`: `writeJson(string,string)` _suggested: keep as is_
- [ ] `writeJson_1`: `writeJson(string,string,string)` _suggested: keep as is_
- [ ] `writeLine`: `writeLine(string,string)` _suggested: keep as is_
- [ ] `writeToml_0`: `writeToml(string,string)` _suggested: keep as is_
- [ ] `writeToml_1`: `writeToml(string,string,string)` _suggested: keep as is_
