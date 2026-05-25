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

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract UnprotectedReinitializer is Initializable {
    uint256 public fee;

    function initializeV2(uint256 fee_) external reinitializer(2) { //~WARN: upgradeable initializer is not protected against direct implementation calls
        fee = fee_;
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract UnprotectedNamedInitialize is Initializable {
    address public admin;

    function initialize(address admin_) external {
        admin = admin_;
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
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

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract UnprotectedArrayWrite is Initializable {
    address[] public owners;

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        owners.push(owner_);
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract NoDestructiveSink is Initializable {
    address public owner;

    function initialize(address owner_) public initializer {
        owner = owner_;
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

contract ConstructorInitializerReinitializer is Initializable {
    uint256 public fee;

    constructor() initializer {}

    function initializeV2(uint256 fee_) external reinitializer(2) { //~WARN: upgradeable initializer is not protected against direct implementation calls
        fee = fee_;
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
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

library UnrelatedInitializerLock {
    function _disableInitializers() internal pure {}
}

contract UnrelatedInitializerLockCall is Initializable {
    address public owner;

    constructor() {
        UnrelatedInitializerLock._disableInitializers();
    }

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        owner = owner_;
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
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

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract OnlyProxySink is Initializable {
    address public owner;

    modifier onlyProxy() {
        _;
    }

    function initialize(address owner_) external initializer {
        owner = owner_;
    }

    function execute(address target, bytes calldata data) external onlyProxy {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract DangerousBase {
    function execute(address target, bytes calldata data) external virtual {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract OverridesDangerousBase is Initializable, DangerousBase {
    address public owner;

    function initialize(address owner_) external initializer {
        owner = owner_;
    }

    function execute(address, bytes calldata) external pure override {}
}

contract OverloadedDangerousBase is Initializable, DangerousBase {
    address public owner;

    function initialize(address owner_) external initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        owner = owner_;
    }

    function execute(uint256, bytes calldata) external pure {}
}

contract NoStateWriteInitializer is Initializable {
    event Initialized(address indexed account);

    function initialize(address account) external initializer {
        emit Initialized(account);
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract NonUpgradeableInitializeName {
    address public owner;

    function initialize(address owner_) external {
        owner = owner_;
    }
}

contract InheritedInitializerBase is Initializable {
    address public owner;

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        owner = owner_;
    }
}

contract InheritedInitializerDerived is InheritedInitializerBase {
    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}

contract StorageAliasInitializer is Initializable {
    struct Layout {
        address owner;
    }

    bytes32 private constant SLOT = keccak256("foundry.storage.alias.initializer");

    function _getLayout() private pure returns (Layout storage s) {
        bytes32 slot = SLOT;
        assembly {
            s.slot := slot
        }
    }

    function initialize(address owner_) public initializer { //~WARN: upgradeable initializer is not protected against direct implementation calls
        Layout storage s = _getLayout();
        s.owner = owner_;
    }

    function execute(address target, bytes calldata data) external {
        (bool ok,) = target.delegatecall(data);
        require(ok);
    }
}
