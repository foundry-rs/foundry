//@compile-flags: --only-lint protected-vars

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract ExactOverloads {
    /// @custom:security write-protection="guard(uint256)"
    uint256 protectedValue;

    function guard(uint256) internal pure {}

    function guard(address) internal pure {}

    function unsafeWrongOverload() external { //~WARN: protected variable `protectedValue` is written without `guard(uint256)`
        guard(address(0));
        protectedValue = 1;
    }

    function safeExactOverload() external {
        guard(1);
        protectedValue = 2;
    }
}

contract VirtualGuardBase {
    /// @custom:security write-protection="guard()"
    uint256 internal protectedValue;

    function guard() internal pure {}

    function hook() internal virtual {
        guard();
    }

    function writeThroughHook() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        hook();
        protectedValue = 1;
    }
}

contract VirtualGuardRemoved is VirtualGuardBase {
    function hook() internal pure override {}
}

contract VirtualGuardAddedBase {
    /// @custom:security write-protection="guard()"
    uint256 internal protectedValue;

    function guard() internal pure {}

    function hook() internal virtual {}

    function writeThroughHook() external {
        hook();
        protectedValue = 1;
    }
}

contract VirtualGuardAdded is VirtualGuardAddedBase {
    function hook() internal pure override {
        guard();
    }
}

contract ModifierGuardBase {
    /// @custom:security write-protection="guard()"
    uint256 internal protectedValue;

    function guard() internal pure {}

    modifier checked() virtual {
        guard();
        _;
    }

    function writeWithModifier() external checked { //~WARN: protected variable `protectedValue` is written without `guard()`
        protectedValue = 1;
    }
}

contract ModifierGuardRemoved is ModifierGuardBase {
    modifier checked() override {
        _;
    }
}

contract ModifierGuardAddedBase {
    /// @custom:security write-protection="guard()"
    uint256 internal protectedValue;

    function guard() internal pure {}

    modifier checked() virtual {
        _;
    }

    function writeWithModifier() external checked {
        protectedValue = 1;
    }
}

contract ModifierGuardAdded is ModifierGuardAddedBase {
    modifier checked() override {
        guard();
        _;
    }
}

struct LibrarySettings {
    uint256 value;
}

library SettingsLib {
    function mutate(LibrarySettings storage settings) internal {
        settings.value = 1;
    }
}

contract AttachedLibraryWrite {
    using SettingsLib for LibrarySettings;

    /// @custom:security write-protection="guard()"
    LibrarySettings protectedSettings;

    function guard() internal pure {}

    function unsafeWrite() external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        protectedSettings.mutate();
    }
}

contract AdvancedStorageAliases {
    struct Settings {
        uint256 value;
    }

    /// @custom:security write-protection="guard()"
    Settings protectedSettings;
    Settings otherSettings;

    /// @custom:security write-protection="guard()"
    Settings[] protectedList;

    function guard() internal pure {}

    function settings() internal view returns (Settings storage result) {
        result = protectedSettings;
    }

    function selectedSettings(bool chooseProtected) internal view returns (Settings storage) {
        if (chooseProtected) return protectedSettings;
        return otherSettings;
    }

    function returnedAlias() external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        settings().value = 1;
    }

    function earlyReturnedAlias(bool chooseProtected) external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        selectedSettings(chooseProtected).value = 1;
    }

    function pushedAlias() external { //~WARN: protected variable `protectedList` is written without `guard()`
        protectedList.push().value = 1;
    }

    function conditionalAlias(bool chooseProtected) external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        Settings storage selected = chooseProtected ? protectedSettings : otherSettings;
        selected.value = 1;
    }

    function exitedAlias(bool stop) external {
        Settings storage selected = otherSettings;
        if (stop) {
            selected = protectedSettings;
            return;
        }
        selected.value = 1;
    }

    function reassignOnly() external {
        Settings storage selected = protectedSettings;
        selected = otherSettings;
    }

    function loopAlias() external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        Settings storage first = otherSettings;
        Settings storage second = otherSettings;
        for (uint256 i; i < 2; ++i) {
            first = second;
            second = protectedSettings;
        }
        first.value = 1;
    }

    function breakAlias(bool shouldIterate, bool shouldBreak) external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        Settings storage selected = otherSettings;
        while (shouldIterate) {
            selected = protectedSettings;
            if (shouldBreak) break;
            selected = otherSettings;
            shouldIterate = false;
        }
        selected.value = 1;
    }

    function continueAlias(bool shouldIterate, bool shouldContinue) external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        Settings storage selected = otherSettings;
        while (shouldIterate) {
            selected = protectedSettings;
            if (shouldContinue) {
                shouldIterate = false;
                continue;
            }
            selected = otherSettings;
            shouldIterate = false;
        }
        selected.value = 1;
    }

    function recurse(Settings storage current, bool again) internal {
        if (again) recurse(protectedSettings, false);
        current.value = 1;
    }

    function recursiveAlias() external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        recurse(otherSettings, true);
    }

    function mutate(Settings storage safe, Settings storage target) internal {
        if (safe.value == type(uint256).max) revert();
        target.value = 1;
    }

    function namedArguments() external { //~WARN: protected variable `protectedSettings` is written without `guard()`
        mutate({target: protectedSettings, safe: otherSettings});
    }
}

contract SignatureType {}

contract ModifierSourceSignature {
    /// @custom:security write-protection="modifierGuard(SignatureType)"
    uint256 protectedValue;

    modifier modifierGuard(SignatureType) {
        _;
    }

    function safeWrite(SignatureType value) external modifierGuard(value) {
        protectedValue = 1;
    }

    function unsafeWrite() external { //~WARN: protected variable `protectedValue` is written without `modifierGuard(SignatureType)`
        protectedValue = 2;
    }
}

contract ModifierFunctionSignature {
    /// @custom:security write-protection="modifierGuard(function(uint256) returns(bool))"
    uint256 protectedValue;

    /// @custom:security write-protection="functionGuard(function(uint256) returns(bool))"
    uint256 functionProtected;

    modifier modifierGuard(function(uint256) external returns (bool)) {
        _;
    }

    function functionGuard(function(uint256) external returns (bool)) internal pure {}

    function callback(uint256) external pure returns (bool) {
        return true;
    }

    function safeWrite() external modifierGuard(this.callback) {
        protectedValue = 1;
    }

    function unsafeWrite() external { //~WARN: protected variable `protectedValue` is written without `modifierGuard(function(uint256) returns(bool))`
        protectedValue = 2;
    }

    function safeFunctionWrite() external {
        functionGuard(this.callback);
        functionProtected = 1;
    }

    function unsafeFunctionWrite() external { //~WARN: protected variable `functionProtected` is written without `functionGuard(function(uint256) returns(bool))`
        functionProtected = 2;
    }
}

contract AssemblyWrites {
    /// @custom:security write-protection="guard()"
    uint256 protectedValue;

    function guard() internal pure {}

    function directWrite() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        assembly {
            sstore(protectedValue.slot, 1)
        }
    }

    function helperParameterWrite() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        assembly {
            function write(slot) {
                sstore(slot, 1)
            }
            write(protectedValue.slot)
        }
    }

    function helperReturnWrite() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        assembly {
            let root := protectedValue.slot
            function getSlot(input) -> slot {
                slot := input
            }
            sstore(getSlot(root), 1)
        }
    }

    function multiReturnDeclaration() external {
        assembly {
            function pair(first, second) -> a, b {
                a := first
                b := second
            }
            let protectedSlot, ordinarySlot := pair(protectedValue.slot, 0)
            sstore(ordinarySlot, 1)
        }
    }

    function multiReturnDeclarationWrite() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        assembly {
            function pair(first, second) -> a, b {
                a := first
                b := second
            }
            let protectedSlot, ordinarySlot := pair(protectedValue.slot, 0)
            sstore(protectedSlot, 1)
        }
    }

    function multiReturnAssignment() external {
        assembly {
            function pair(first, second) -> a, b {
                a := first
                b := second
            }
            let protectedSlot := 0
            let ordinarySlot := 0
            protectedSlot, ordinarySlot := pair(protectedValue.slot, 0)
            sstore(ordinarySlot, 1)
        }
    }

    function multiReturnAssignmentWrite() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        assembly {
            function pair(first, second) -> a, b {
                a := first
                b := second
            }
            let protectedSlot := 0
            let ordinarySlot := 0
            protectedSlot, ordinarySlot := pair(protectedValue.slot, 0)
            sstore(protectedSlot, 1)
        }
    }
}

contract SharedInheritedEntry {
    /// @custom:security write-protection="guard()"
    uint256 protectedValue;

    function guard() internal pure {}

    function unsafeWrite() external {
        //~^WARN: protected variable `protectedValue` is written without `guard()` in most-derived contract `SharedLeafOne`
        //~|WARN: protected variable `protectedValue` is written without `guard()` in most-derived contract `SharedLeafTwo`
        protectedValue = 1;
    }
}

contract SharedLeafOne is SharedInheritedEntry {}

contract SharedLeafTwo is SharedInheritedEntry {}

contract SpecialEntryPoints {
    /// @custom:security write-protection="guard()"
    uint256 protectedValue;

    function guard() internal pure {}

    fallback() external { //~WARN: protected variable `protectedValue` is written without `guard()`
        protectedValue = 1;
    }

    receive() external payable { //~WARN: protected variable `protectedValue` is written without `guard()`
        protectedValue = 2;
    }
}
