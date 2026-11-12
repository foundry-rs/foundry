//! Cheatcode authorization for direct and delegated execution on a local fork.

use super::*;

#[forgetest]
async fn fork_cheatcode_access_compatibility(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    prj.add_test("CheatcodeAccess.t.sol", CHEATCODE_ACCESS_COMPATIBILITY);
    cmd.unset_env("FOUNDRY_ACCESS_REGRESSION_UNUSED");
    cmd.env("FOUNDRY_ISOLATE", "false");
    cmd.args([
        "test",
        "--fork-url",
        &handle.http_endpoint(),
        "--evm-version",
        "prague",
        "--match-contract",
        "CheatcodeAccessCompatibilityTest",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 12 tests for test/CheatcodeAccess.t.sol:CheatcodeAccessCompatibilityTest
[PASS] testForkAuthorizationIsReadAtCheatcodeUse() ([GAS])
[PASS] testForkCallbacksRetainChildAccess() ([GAS])
[PASS] testForkConstructorsRetainAccessWithCallerOverrides() ([GAS])
[PASS] testForkDelegatedCodeRequiresAuthorization() ([GAS])
[PASS] testForkDelegatedConstructorsCannotUseCheatcodes() ([GAS])
[PASS] testForkDelegatedCreationDoesNotAuthorizeChildren() ([GAS])
[PASS] testForkEip7702RequiresAuthorizedContextAndImplementation() ([GAS])
[PASS] testForkEnvHandoffChecksCreatorCode() ([GAS])
[PASS] testForkExplicitAuthorizationAllowsDirectAndDelegatedCalls() ([GAS])
[PASS] testForkExplicitFactoryAuthorizationAllowsChildren() ([GAS])
[PASS] testForkTrustedConstructorRetainsAccessAfterEnvChange() ([GAS])
[PASS] testForkTrustedFactoryRetainsChildAccess() ([GAS])
Suite result: ok. 12 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 12 tests passed, 0 failed, 0 skipped (12 total tests)

"#]]);
}

#[forgetest]
async fn fork_cheatcode_access_in_isolation(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    prj.add_test("CheatcodeAccess.t.sol", CHEATCODE_ACCESS_COMPATIBILITY);
    cmd.unset_env("FOUNDRY_ACCESS_REGRESSION_UNUSED");
    cmd.args([
        "test",
        "--fork-url",
        &handle.http_endpoint(),
        "--evm-version",
        "prague",
        "--isolate",
        "--match-contract",
        "CheatcodeAccessCompatibilityTest",
        "--match-test",
        "testFork(DelegatedConstructorsCannotUseCheatcodes|DelegatedCreationDoesNotAuthorizeChildren|Eip7702RequiresAuthorizedContextAndImplementation|EnvHandoffChecksCreatorCode|TrustedConstructorRetainsAccessAfterEnvChange)",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 5 tests for test/CheatcodeAccess.t.sol:CheatcodeAccessCompatibilityTest
[PASS] testForkDelegatedConstructorsCannotUseCheatcodes() ([GAS])
[PASS] testForkDelegatedCreationDoesNotAuthorizeChildren() ([GAS])
[PASS] testForkEip7702RequiresAuthorizedContextAndImplementation() ([GAS])
[PASS] testForkEnvHandoffChecksCreatorCode() ([GAS])
[PASS] testForkTrustedConstructorRetainsAccessAfterEnvChange() ([GAS])
Suite result: ok. 5 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 5 tests passed, 0 failed, 0 skipped (5 total tests)

"#]]);
}

#[forgetest]
fn nonfork_cheatcode_access_compatibility(prj: _, cmd: _) {
    prj.add_test("CheatcodeAccess.t.sol", CHEATCODE_ACCESS_COMPATIBILITY);
    cmd.unset_env("FOUNDRY_ACCESS_REGRESSION_UNUSED");
    cmd.args(["test", "--evm-version", "prague", "--match-contract", "CheatcodeAccessNonForkTest"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CheatcodeAccess.t.sol:CheatcodeAccessNonForkTest
[PASS] testNonForkAccessRetainsUnrestrictedBehavior() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
}

#[forgetest]
async fn fork_create2_redirect_preserves_cheatcode_access(prj: _, cmd: _) {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_code(
        foundry_evm::constants::DEFAULT_CREATE2_DEPLOYER,
        Bytes::from_static(foundry_evm::constants::DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE),
    )
    .await
    .unwrap();
    prj.add_test("Create2Access.t.sol", CREATE2_REDIRECT_ACCESS);
    for rewrite in [false, true] {
        prj.update_config(|config| config.always_use_create_2_factory = rewrite);
        for isolate in ["false", "true"] {
            cmd.forge_fuse();
            cmd.unset_env("FOUNDRY_ACCESS_REGRESSION_UNUSED");
            cmd.unset_env("FOUNDRY_ALWAYS_USE_CREATE_2_FACTORY");
            cmd.env("FOUNDRY_ISOLATE", isolate);
            cmd.args([
                "test",
                "--fork-url",
                &handle.http_endpoint(),
                "--evm-version",
                "prague",
                "--match-contract",
                "Create2RedirectTest",
                "-j1",
            ])
            .assert_success()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/Create2Access.t.sol:AuthorizedIssuerAuthorizedFactoryCreate2RedirectTest
[PASS] testForkCreate2RedirectAuthorization() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/Create2Access.t.sol:AuthorizedIssuerUntrustedFactoryCreate2RedirectTest
[PASS] testForkCreate2RedirectAuthorization() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/Create2Access.t.sol:UntrustedIssuerAuthorizedFactoryCreate2RedirectTest
[PASS] testForkCreate2RedirectAuthorization() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/Create2Access.t.sol:UntrustedIssuerUntrustedFactoryCreate2RedirectTest
[PASS] testForkCreate2RedirectAuthorization() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 4 test suites [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]);
        }
    }
}

const CHEATCODE_ACCESS_COMPATIBILITY: &str = r#"
interface Vm {
    function prank(address sender) external;
    function startBroadcast(address sender) external;
    function stopBroadcast() external;
    function allowCheatcodes(address account) external;
    function chainId(uint256 newChainId) external;
    function deployCode(string calldata artifactPath) external returns (address);
    function computeCreateAddress(address deployer, uint256 nonce) external pure returns (address);
    function computeCreate2Address(bytes32 salt, bytes32 initCodeHash, address deployer) external pure returns (address);
    function envOr(string calldata name, uint256 defaultValue) external view returns (uint256);
    function etch(address target, bytes calldata code) external;
    function getNonce(address account) external view returns (uint64);
    function toString(address account) external pure returns (string memory);
}

interface Authorizer {
    function authorizeReader() external;
    function changeChainId(uint256 newChainId) external;
}

interface CallbackCreator {
    function createCallbackReader(uint256 mode) external returns (address);
}

abstract contract UsesVm {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function readDefault() internal view returns (uint256) {
        return vm.envOr("FOUNDRY_ACCESS_REGRESSION_UNUSED", uint256(7));
    }

    function denialHash(address account) internal pure returns (bytes32) {
        string memory reason = string.concat(
            "cheatcodes are not enabled for ", vm.toString(account),
            "; see `vm.allowCheatcodes(address)`"
        );
        return keccak256(abi.encodeWithSignature("CheatcodeError(string)", reason));
    }
}

contract EnvReader is UsesVm {
    function read() external view returns (uint256) {
        return readDefault();
    }

    function requestAuthorization() external returns (uint256) {
        Authorizer(address(this)).authorizeReader();
        return readDefault();
    }
}

contract ConstructorReader is EnvReader {
    uint256 public constructorValue;

    constructor() {
        constructorValue = readDefault();
    }
}

contract DeployCodeConstructorReader is EnvReader {
    constructor() {
        require(readDefault() == 7, "handoff constructor access lost");
        address child = vm.deployCode("CheatcodeAccess.t.sol:ConstructorReader");
        require(ConstructorReader(child).constructorValue() == 7, "nested deployCode constructor access lost");
        require(EnvReader(child).read() == 7, "nested deployCode child access lost");
    }
}

contract EnvFactory {
    function create(bool salted, bool readInConstructor, bytes32 salt) public returns (address) {
        if (readInConstructor) {
            return salted
                ? address(new ConstructorReader{salt: salt}())
                : address(new ConstructorReader());
        }
        return salted ? address(new EnvReader{salt: salt}()) : address(new EnvReader());
    }

    function createAfterChainIdChange(bool salted, bool readInConstructor, bytes32 salt, uint256 newChainId)
        external returns (address)
    {
        Authorizer(address(this)).changeChainId(newChainId);
        return create(salted, readInConstructor, salt);
    }

    function createThrough(EnvFactory factory) external returns (address) {
        return factory.create(false, true, bytes32(0));
    }
}

library EnvLibrary {
    function read() external view returns (uint256) {
        return Vm(address(uint160(uint256(keccak256("hevm cheat code")))))
            .envOr("FOUNDRY_ACCESS_REGRESSION_UNUSED", uint256(7));
    }
}

contract CallbackPool {
    function callback(uint256 mode) external returns (address) {
        return CallbackCreator(msg.sender).createCallbackReader(mode);
    }
}

contract CallbackPoolProxy {
    function callback(uint256 mode) external returns (address) {
        (bool ok, bytes memory output) = address(0xA003).delegatecall(
            abi.encodeCall(CallbackPool.callback, (mode))
        );
        require(ok, "pool proxy callback failed");
        return abi.decode(output, (address));
    }
}

abstract contract AccessFixture is UsesVm {
    address constant READER = address(0x1234);
    address constant FACTORY = address(0x5678);
    address constant DELEGATOR = address(0x9012);

    function setUp() public {
        vm.etch(READER, type(EnvReader).runtimeCode);
        vm.etch(FACTORY, type(EnvFactory).runtimeCode);
    }

    function expectDenied(bool ok, bytes memory output, address account) internal pure {
        require(!ok, "cheatcode access unexpectedly allowed");
        require(keccak256(output) == denialHash(account), "unexpected authorization error");
    }

    function expectValue(bool ok, bytes memory output) internal pure {
        require(ok, "expected successful call");
        require(abi.decode(output, (uint256)) == 7, "unexpected default");
    }

    function invoke(bool useCallcode, address target, bytes memory input)
        internal returns (bool ok, bytes memory output)
    {
        if (!useCallcode) return target.delegatecall(input);
        assembly {
            ok := callcode(gas(), target, 0, add(input, 32), mload(input), 0, 0)
            output := mload(0x40)
            mstore(output, returndatasize())
            returndatacopy(add(output, 32), 0, returndatasize())
            mstore(0x40, and(add(add(output, returndatasize()), 63), not(31)))
        }
    }

    function creationInput(uint256 mode, bool readInConstructor) internal pure returns (bytes memory) {
        return abi.encodeCall(EnvFactory.create, (mode % 2 == 1, readInConstructor, bytes32(mode)));
    }
}

contract CheatcodeAccessCompatibilityTest is AccessFixture {
    uint256 public constructorValue;
    address callbackPool;

    constructor() {
        constructorValue = readDefault();
    }

    function authorizeReader() external {
        vm.allowCheatcodes(READER);
    }

    function changeChainId(uint256 newChainId) external {
        require(msg.sender == address(this), "unexpected chain ID callback sender");
        vm.chainId(newChainId);
    }

    function createCallbackReader(uint256 mode) external returns (address) {
        require(msg.sender == callbackPool, "unexpected callback sender");
        require(readDefault() == 7, "callback access lost");
        bytes32 salt = keccak256(abi.encode(msg.sender, mode));
        EnvReader reader;
        if (mode >= 2) {
            ConstructorReader constructed = mode % 2 == 1
                ? new ConstructorReader{salt: salt}()
                : new ConstructorReader();
            require(constructed.constructorValue() == 7, "callback constructor access lost");
            reader = constructed;
        } else {
            reader = mode % 2 == 1 ? new EnvReader{salt: salt}() : new EnvReader();
        }
        require(reader.read() == 7, "callback child runtime access lost");
        return address(reader);
    }

    function testForkAuthorizationIsReadAtCheatcodeUse() public {
        (bool ok, bytes memory output) = READER.delegatecall(abi.encodeCall(EnvReader.requestAuthorization, ()));
        expectValue(ok, output);
    }

    function testForkCallbacksRetainChildAccess() public {
        address implementation = address(0xA003);
        vm.etch(implementation, type(CallbackPool).runtimeCode);
        vm.etch(address(0xA002), type(CallbackPoolProxy).runtimeCode);
        address[2] memory pools = [address(0xA002), address(new CallbackPoolProxy())];
        for (uint256 i; i < pools.length; i++) {
            callbackPool = pools[i];
            for (uint256 mode; mode < 4; mode++) {
                address child = CallbackPool(callbackPool).callback(mode);
                require(EnvReader(child).read() == 7, "returned callback child access lost");
            }
        }
    }

    function testForkDelegatedCreationDoesNotAuthorizeChildren() public {
        for (uint256 mode; mode < 4; mode++) {
            (bool created, bytes memory result) = invoke(mode >= 2, FACTORY, creationInput(mode, false));
            require(created, "runtime-only child creation failed");
            address child = abi.decode(result, (address));
            (bool ok, bytes memory output) = child.call(abi.encodeCall(EnvReader.read, ()));
            expectDenied(ok, output, child);
            require(readDefault() == 7, "parent access lost after create");
        }
        address directChild = EnvFactory(FACTORY).create(false, false, bytes32(0));
        (bool directOk, bytes memory directOutput) = directChild.call(abi.encodeCall(EnvReader.read, ()));
        expectDenied(directOk, directOutput, directChild);
    }

    function testForkDelegatedConstructorsCannotUseCheatcodes() public {
        for (uint256 mode; mode < 4; mode++) {
            address child = mode % 2 == 1
                ? vm.computeCreate2Address(bytes32(mode), keccak256(type(ConstructorReader).creationCode), address(this))
                : vm.computeCreateAddress(address(this), vm.getNonce(address(this)));
            (bool ok, bytes memory output) = invoke(mode >= 2, FACTORY, creationInput(mode, true));
            expectDenied(ok, output, child);
            require(readDefault() == 7, "parent access lost after constructor revert");
        }
    }

    function testForkEip7702RequiresAuthorizedContextAndImplementation() public {
        vm.etch(DELEGATOR, abi.encodePacked(hex"ef0100", READER));
        vm.allowCheatcodes(DELEGATOR);
        (bool ok, bytes memory output) = DELEGATOR.call(abi.encodeCall(EnvReader.read, ()));
        expectDenied(ok, output, READER);

        vm.allowCheatcodes(READER);
        (ok, output) = DELEGATOR.call(abi.encodeCall(EnvReader.read, ()));
        expectValue(ok, output);

        address otherDelegator = address(0x9013);
        vm.etch(otherDelegator, abi.encodePacked(hex"ef0100", READER));
        (ok, output) = otherDelegator.call(abi.encodeCall(EnvReader.read, ()));
        expectDenied(ok, output, otherDelegator);
        require(readDefault() == 7, "parent access lost after delegation designator");
    }

    function testForkConstructorsRetainAccessWithCallerOverrides() public {
        vm.prank(address(0xbeef));
        ConstructorReader pranked = new ConstructorReader();
        require(pranked.constructorValue() == 7, "pranked constructor access lost");
        require(pranked.read() == 7, "pranked child access lost");

        vm.startBroadcast(address(0xcafe));
        ConstructorReader broadcasted = new ConstructorReader();
        vm.stopBroadcast();
        require(broadcasted.constructorValue() == 7, "broadcast constructor access lost");
        require(broadcasted.read() == 7, "broadcast child access lost");
        require(readDefault() == 7, "parent access lost after caller overrides");
    }

    function testForkExplicitAuthorizationAllowsDirectAndDelegatedCalls() public {
        require(constructorValue == 7, "test constructor access lost");
        require(EnvLibrary.read() == 7, "local library access lost");
        (bool ok, bytes memory output) = READER.call(abi.encodeCall(EnvReader.read, ()));
        expectDenied(ok, output, READER);
        vm.allowCheatcodes(READER);
        (ok, output) = READER.call(abi.encodeCall(EnvReader.read, ()));
        expectValue(ok, output);
        for (uint256 mode; mode < 2; mode++) {
            (ok, output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.read, ()));
            expectValue(ok, output);
            require(readDefault() == 7, "parent access lost after authorized return");
        }
    }

    function testForkExplicitFactoryAuthorizationAllowsChildren() public {
        vm.allowCheatcodes(FACTORY);
        for (uint256 mode; mode < 4; mode++) {
            (bool ok, bytes memory output) = invoke(mode >= 2, FACTORY, creationInput(mode, true));
            require(ok, "authorized factory creation failed");
            ConstructorReader child = ConstructorReader(abi.decode(output, (address)));
            require(child.constructorValue() == 7, "authorized constructor access lost");
            require(child.read() == 7, "authorized child access lost");
        }
    }

    function testForkDelegatedCodeRequiresAuthorization() public {
        for (uint256 mode; mode < 2; mode++) {
            (bool ok, bytes memory output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.read, ()));
            expectDenied(ok, output, READER);
            require(readDefault() == 7, "parent access lost after denied access");
        }
    }

    function testForkEnvHandoffChecksCreatorCode() public {
        for (uint256 mode; mode < 8; mode++) {
            bool salted = mode % 2 == 1;
            bool readInConstructor = mode % 4 >= 2;
            address child = salted
                ? vm.computeCreate2Address(
                    bytes32(mode),
                    keccak256(readInConstructor ? type(ConstructorReader).creationCode : type(EnvReader).creationCode),
                    address(this)
                )
                : vm.computeCreateAddress(address(this), vm.getNonce(address(this)));
            (bool ok, bytes memory output) = invoke(mode >= 4, FACTORY, abi.encodeCall(
                EnvFactory.createAfterChainIdChange, (salted, readInConstructor, bytes32(mode), block.chainid + mode + 1)
            ));
            if (readInConstructor) {
                expectDenied(ok, output, child);
            } else {
                require(ok && abi.decode(output, (address)) == child, "handoff child creation failed");
                (ok, output) = child.call(abi.encodeCall(EnvReader.read, ()));
                expectDenied(ok, output, child);
            }
        }
        address deployed = vm.deployCode("CheatcodeAccess.t.sol:ConstructorReader");
        require(ConstructorReader(deployed).constructorValue() == 7, "stale creator blocked deployCode");
    }

    function testForkTrustedConstructorRetainsAccessAfterEnvChange() public {
        vm.chainId(block.chainid + 1);
        DeployCodeConstructorReader child = new DeployCodeConstructorReader();
        require(child.read() == 7, "handoff child runtime access lost");
    }

    function testForkTrustedFactoryRetainsChildAccess() public {
        EnvFactory factory = new EnvFactory();
        bytes memory input = abi.encodeCall(EnvFactory.createThrough, (factory));
        for (uint256 mode; mode < 3; mode++) {
            bool ok;
            bytes memory output;
            if (mode == 0) (ok, output) = FACTORY.call(input);
            else (ok, output) = invoke(mode == 2, FACTORY, input);
            require(ok, "trusted factory creation failed");
            ConstructorReader child = ConstructorReader(abi.decode(output, (address)));
            require(child.constructorValue() == 7, "trusted factory constructor access lost");
            require(child.read() == 7, "trusted factory child access lost");
        }
    }
}

contract CheatcodeAccessNonForkTest is AccessFixture {
    function testNonForkAccessRetainsUnrestrictedBehavior() public {
        require(EnvReader(READER).read() == 7, "nonfork direct access lost");
        for (uint256 mode; mode < 4; mode++) {
            (bool ok, bytes memory output) = invoke(mode >= 2, READER, abi.encodeCall(EnvReader.read, ()));
            expectValue(ok, output);
            (ok, output) = invoke(mode >= 2, FACTORY, creationInput(mode, true));
            require(ok, "nonfork child creation failed");
            ConstructorReader child = ConstructorReader(abi.decode(output, (address)));
            require(child.constructorValue() == 7, "nonfork constructor access lost");
            require(child.read() == 7, "nonfork child access lost");
        }
        vm.etch(DELEGATOR, abi.encodePacked(hex"ef0100", READER));
        require(EnvReader(DELEGATOR).read() == 7, "nonfork delegation designator access lost");
    }
}

"#;

const CREATE2_REDIRECT_ACCESS: &str = r#"
interface Vm {
    function prank(address sender) external;
    function etch(address target, bytes calldata code) external;
    function allowCheatcodes(address target) external;
    function chainId(uint256 newChainId) external;
    function envOr(string calldata name, uint256 defaultValue) external view returns (uint256);
}

abstract contract UsesVm {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function readDefault() internal view returns (uint256) {
        return vm.envOr("FOUNDRY_ACCESS_REGRESSION_UNUSED", uint256(7));
    }
}

contract EnvReader is UsesVm {
    function read() external view returns (uint256) {
        return readDefault();
    }
}

contract ConstructorReader is EnvReader {
    uint256 public constructorValue;

    constructor() {
        constructorValue = readDefault();
    }
}

contract Create2EnvChanger is UsesVm {
    function change() external {
        vm.chainId(block.chainid + 1);
    }
}

contract Create2RedirectImplementation is UsesVm {
    address constant CREATE2_FACTORY = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    Create2EnvChanger internal changer;
    bool internal issuerAuthorized;
    bool internal factoryAuthorized;

    function testForkCreate2RedirectAuthorization() public {
        for (uint256 mode; mode < 4; mode++) {
            if (mode >= 2) changer.change();
            if (issuerAuthorized && mode % 2 == 1) vm.prank(address(0xbeef));
            bytes32 salt = bytes32(mode);
            if (mode % 2 == 0) {
                EnvReader child = new EnvReader{salt: salt}();
                checkAccess(child, issuerAuthorized);
            } else {
                bool created;
                try new ConstructorReader{salt: salt}() returns (ConstructorReader child) {
                    created = true;
                    require(child.constructorValue() == 7, "constructor value lost");
                    checkAccess(child, true);
                } catch {}
                require(created == issuerAuthorized, "constructor authorization changed");
            }

            // Explicit factory calls retain their own authorization after successful or failed creates.
            (bool ok, bytes memory output) = CREATE2_FACTORY.call(
                abi.encodePacked(bytes32(mode + 4), type(EnvReader).creationCode)
            );
            require(ok && output.length == 20, "explicit factory creation failed");
            checkAccess(EnvReader(address(bytes20(output))), factoryAuthorized);
        }
    }

    function checkAccess(EnvReader child, bool authorized) internal {
        (bool ok, bytes memory output) = address(child).call(abi.encodeCall(EnvReader.read, ()));
        require(ok == authorized, "runtime authorization changed");
        if (ok) require(abi.decode(output, (uint256)) == 7, "runtime value lost");
    }
}

abstract contract Create2RedirectFixture is Create2RedirectImplementation {
    function setUpImplementation(bool authorized, bool authorizeFactory) internal {
        factoryAuthorized = authorizeFactory;
        if (factoryAuthorized) vm.allowCheatcodes(CREATE2_FACTORY);
        changer = new Create2EnvChanger();
        issuerAuthorized = authorized;
        address implementation = address(0x1234);
        vm.etch(implementation, type(Create2RedirectImplementation).runtimeCode);
        if (authorized) vm.allowCheatcodes(implementation);
        // EIP-7702 keeps the issuing implementation at the root, where automatic rewriting applies.
        vm.etch(address(this), abi.encodePacked(hex"ef0100", implementation));
    }
}

contract AuthorizedIssuerAuthorizedFactoryCreate2RedirectTest is Create2RedirectFixture {
    function setUp() public {
        setUpImplementation(true, true);
    }
}

contract AuthorizedIssuerUntrustedFactoryCreate2RedirectTest is Create2RedirectFixture {
    function setUp() public {
        setUpImplementation(true, false);
    }
}

contract UntrustedIssuerAuthorizedFactoryCreate2RedirectTest is Create2RedirectFixture {
    function setUp() public {
        setUpImplementation(false, true);
    }
}

contract UntrustedIssuerUntrustedFactoryCreate2RedirectTest is Create2RedirectFixture {
    function setUp() public {
        setUpImplementation(false, false);
    }
}

"#;
