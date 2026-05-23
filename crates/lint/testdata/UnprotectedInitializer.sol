//@compile-flags: --only-lint unprotected-initializer

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

abstract contract Initializable {
    bool private _initialized;

    modifier initializer() {
        _;
    }

    modifier reinitializer(uint64) {
        _;
    }

    modifier onlyInitializing() {
        _;
    }

    function _disableInitializers() internal {
        _initialized = true;
    }
}

contract UnprotectedInitializer is Initializable {
    address public owner;

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        owner = owner_;
    }
}

contract UnprotectedReinitializer is Initializable {
    uint256 public fee;

    function initializeV2(uint256 fee_) external reinitializer(2) { //~WARN: upgradeable initializer is not protected against direct implementation calls
        fee = fee_;
    }
}

contract UnprotectedNamedInitialize is Initializable {
    address public admin;

    function initialize(address admin_) external { //~WARN: upgradeable initializer is not protected against direct implementation calls
        admin = admin_;
    }
}

contract UnprotectedInternalWrite is Initializable {
    address public owner;

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        _setOwner(owner_);
    }

    function _setOwner(address owner_) internal {
        owner = owner_;
    }
}

contract UnprotectedArrayWrite is Initializable {
    address[] public owners;

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        owners.push(owner_);
    }
}

contract ProtectedDisableInitializers is Initializable {
    address public owner;

    constructor() {
        _disableInitializers();
    }

    function initialize(address owner_) public initializer {
        owner = owner_;
    }
}

contract ProtectedConstructorInitializer is Initializable {
    address public owner;

    constructor() initializer {}

    function initialize(address owner_) public initializer {
        owner = owner_;
    }
}

contract ProtectedThroughHelper is Initializable {
    address public owner;

    constructor() {
        _lockImplementation();
    }

    function _lockImplementation() internal {
        _disableInitializers();
    }

    function initialize(address owner_) public initializer {
        owner = owner_;
    }
}

contract OnlyProxyInitializer is Initializable {
    address public owner;

    modifier onlyProxy() {
        _;
    }

    function initialize(address owner_) external initializer onlyProxy {
        owner = owner_;
    }
}

contract NoStateWriteInitializer is Initializable {
    event Initialized(address indexed account);

    function initialize(address account) external initializer {
        emit Initialized(account);
    }
}

contract NonUpgradeableInitializeName {
    address public owner;

    function initialize(address owner_) external {
        owner = owner_;
    }
}
