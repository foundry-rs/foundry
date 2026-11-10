//! Cheatcode authorization for direct and delegated execution on a local fork.

use super::*;

#[forgetest]
async fn fork_cheatcode_access_compatibility(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    prj.add_test("CheatcodeAccess.t.sol", CHEATCODE_ACCESS_COMPATIBILITY);
    for isolate in [false, true] {
        for (contract, expected) in [
            (
                "CheatcodeAccessCompatibilityTest",
                str![[r#"
...
Ran 13 tests for test/CheatcodeAccess.t.sol:CheatcodeAccessCompatibilityTest
[PASS] testForkAuthorizationIsReadAtCheatcodeUse() ([GAS])
[PASS] testForkCallbacksRetainChildAccess() ([GAS])
[PASS] testForkConstructorsAndLocalLibrariesRetainAccess() ([GAS])
[PASS] testForkConstructorsRetainAccessWithCallerOverrides() ([GAS])
[PASS] testForkDelegatedConstructorsCannotUseCheatcodes() ([GAS])
[PASS] testForkDelegatedCreationDoesNotAuthorizeChildren() ([GAS])
[PASS] testForkDeployCodeCannotAuthorizeUntrustedChildren() ([GAS])
[PASS] testForkDeployCodeRetainsAccessAfterNestedExecution() ([GAS])
[PASS] testForkEip7702RequiresAuthorizedContextAndImplementation() ([GAS])
[PASS] testForkExplicitAuthorizationAllowsDirectAndDelegatedCalls() ([GAS])
[PASS] testForkExplicitFactoryAuthorizationAllowsChildren() ([GAS])
[PASS] testForkGenericFactoryCannotAuthorizeUntrustedChildren() ([GAS])
[PASS] testForkParentAccessRestoredAfterDelegation() ([GAS])
Suite result: ok. 13 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 13 tests passed, 0 failed, 0 skipped (13 total tests)

"#]],
            ),
            (
                "CheatcodeAccessRootDelegationTest",
                str![[r#"
...
Ran 1 test for test/CheatcodeAccess.t.sol:CheatcodeAccessRootDelegationTest
[PASS] testForkRootDelegationCannotAuthorizeFactoryChildren() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]],
            ),
        ] {
            cmd.forge_fuse().args([
                "test",
                "--fork-url",
                &handle.http_endpoint(),
                "--evm-version",
                "prague",
                "--match-contract",
                contract,
            ]);
            cmd.unset_env("FOUNDRY_ACCESS_REGRESSION_UNUSED");
            cmd.env("FOUNDRY_ISOLATE", isolate.to_string());
            cmd.assert_success().stdout_eq(expected);
        }
    }
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
async fn fork_cheatcode_access_after_create_select_fork(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    prj.add_test("CheatcodeAccess.t.sol", CHEATCODE_ACCESS_COMPATIBILITY);
    cmd.env("FOUNDRY_ACCESS_REGRESSION_FORK_URL", handle.http_endpoint());
    cmd.unset_env("FOUNDRY_ACCESS_REGRESSION_UNUSED");
    cmd.args(["test", "--match-contract", "CheatcodeAccessSelectForkTest"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CheatcodeAccess.t.sol:CheatcodeAccessSelectForkTest
[PASS] testForkAccessCheckedAfterSelectingFork() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
}

const CHEATCODE_ACCESS_COMPATIBILITY: &str = r#"
interface Vm {
    function prank(address sender) external;
    function startBroadcast(address sender) external;
    function stopBroadcast() external;
    function allowCheatcodes(address account) external;
    function createSelectFork(string calldata url) external returns (uint256);
    function envString(string calldata name) external view returns (string memory);
    function deployCode(string calldata artifactPath) external returns (address);
    function deployCode(string calldata artifactPath, bytes calldata constructorArgs) external returns (address);
    function computeCreateAddress(address deployer, uint256 nonce) external pure returns (address);
    function computeCreate2Address(bytes32 salt, bytes32 initCodeHash, address deployer) external pure returns (address);
    function envOr(string calldata name, uint256 defaultValue) external view returns (uint256);
    function etch(address target, bytes calldata code) external;
    function getNonce(address account) external view returns (uint64);
    function toString(address account) external pure returns (string memory);
}

interface Authorizer {
    function authorizeReader() external;
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

    function ping() external pure returns (uint256) {
        return 7;
    }

    function fail() external pure {
        revert("expected child revert");
    }

    function readThrough(EnvReader reader) external view returns (uint256) {
        return reader.read();
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

contract NestedConstructorReader is EnvReader {
    constructor(uint256 mode) {
        require(readDefault() == 7, "nested constructor access lost");
        require(mode != 1, "expected constructor revert");
        if (mode == 2) assembly { invalid() }
    }
}

contract RecursiveConstructorReader is EnvReader {
    constructor(uint256 mode) {
        for (uint256 childMode; childMode < 3; childMode++) {
            try vm.deployCode{gas: 1_000_000}("CheatcodeAccess.t.sol:NestedConstructorReader", abi.encode(childMode)) returns (address child) {
                require(childMode == 0, "nested constructor did not fail");
                require(EnvReader(child).read() == 7, "recursive child runtime access lost");
            } catch {
                require(childMode != 0, "recursive child creation failed");
            }
            require(readDefault() == 7, "constructor access lost after recursive execution");
        }
        require(mode != 1, "expected recursive constructor revert");
        if (mode == 2) assembly { invalid() }
    }
}

contract EnvFactory {
    function create(bool salted, bool readInConstructor, bytes32 salt) external returns (address) {
        if (readInConstructor) {
            return salted
                ? address(new ConstructorReader{salt: salt}())
                : address(new ConstructorReader());
        }
        return salted ? address(new EnvReader{salt: salt}()) : address(new EnvReader());
    }

    function createThrough(EnvFactory factory, bool salted, bytes32 salt) external returns (address) {
        return factory.create(salted, false, salt);
    }

    function deployCodeThrough(EnvFactory factory) external returns (address) {
        return factory.deployCode();
    }

    function deployCode() external returns (address) {
        return Vm(address(uint160(uint256(keccak256("hevm cheat code")))))
            .deployCode("CheatcodeAccess.t.sol:EnvReader");
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
        vm.etch(address(0xA001), type(CallbackPool).runtimeCode);
        vm.etch(address(0xA002), type(CallbackPoolProxy).runtimeCode);
        vm.etch(address(0xA004), abi.encodePacked(hex"ef0100", implementation));
        address[3] memory pools = [address(0xA001), address(0xA002), address(0xA004)];
        for (uint256 i; i < pools.length; i++) {
            callbackPool = pools[i];
            for (uint256 mode; mode < 4; mode++) {
                address child = CallbackPool(callbackPool).callback(mode);
                require(EnvReader(child).read() == 7, "returned callback child access lost");
            }
        }
    }

    function testForkConstructorsAndLocalLibrariesRetainAccess() public {
        require(constructorValue == 7, "test constructor access lost");
        ConstructorReader reader = new ConstructorReader();
        require(reader.constructorValue() == 7, "child constructor access lost");
        require(reader.read() == 7, "child runtime access lost");
        require(EnvLibrary.read() == 7, "local library access lost");
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

        EnvFactory localFactory = new EnvFactory();
        vm.etch(DELEGATOR, abi.encodePacked(hex"ef0100", FACTORY));
        for (uint256 mode; mode < 2; mode++) {
            if (mode == 1) vm.allowCheatcodes(FACTORY);
            (ok, output) = DELEGATOR.call(abi.encodeCall(
                EnvFactory.createThrough, (localFactory, true, bytes32(mode))
            ));
            require(ok, "delegated factory creation failed");
            address child = abi.decode(output, (address));
            (ok, output) = child.call(abi.encodeCall(EnvReader.read, ()));
            if (mode == 0) expectDenied(ok, output, child);
            else expectValue(ok, output);
        }
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
        (bool ok, bytes memory output) = READER.call(abi.encodeCall(EnvReader.read, ()));
        expectDenied(ok, output, READER);
        vm.allowCheatcodes(READER);
        (ok, output) = READER.call(abi.encodeCall(EnvReader.read, ()));
        expectValue(ok, output);
        for (uint256 mode; mode < 2; mode++) {
            (ok, output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.read, ()));
            expectValue(ok, output);
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

    function testForkGenericFactoryCannotAuthorizeUntrustedChildren() public {
        EnvFactory localFactory = new EnvFactory();
        for (uint256 mode; mode < 4; mode++) {
            (bool created, bytes memory result) = invoke(
                mode >= 2, FACTORY,
                abi.encodeCall(EnvFactory.createThrough, (localFactory, mode % 2 == 1, bytes32(mode)))
            );
            require(created, "generic factory creation failed");
            address child = abi.decode(result, (address));
            (bool ok, bytes memory output) = child.call(abi.encodeCall(EnvReader.read, ()));
            expectDenied(ok, output, child);
        }
    }

    function testForkParentAccessRestoredAfterDelegation() public {
        EnvReader localReader = new EnvReader();
        (bool empty,) = address(0x1111).call("");
        require(empty, "empty-code call failed");
        require(readDefault() == 7, "parent access lost after empty-code call");
        (bool precompile, bytes memory digest) = address(2).staticcall(hex"1234");
        require(precompile && abi.decode(digest, (bytes32)) == sha256(hex"1234"), "precompile call failed");
        require(readDefault() == 7, "parent access lost after precompile call");
        for (uint256 mode; mode < 2; mode++) {
            (bool ok, bytes memory output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.ping, ()));
            expectValue(ok, output);
            require(readDefault() == 7, "parent access lost after return");

            (ok, output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.fail, ()));
            require(!ok, "child did not revert");
            require(keccak256(output) == keccak256(abi.encodeWithSignature("Error(string)", "expected child revert")), "wrong child revert");
            require(readDefault() == 7, "parent access lost after revert");

            (ok, output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.readThrough, (localReader)));
            expectValue(ok, output);
            require(readDefault() == 7, "parent access lost after nested call");

            (ok, output) = invoke(mode == 1, READER, abi.encodeCall(EnvReader.read, ()));
            expectDenied(ok, output, READER);
            require(readDefault() == 7, "parent access lost after denied access");
        }
    }

    function testForkDeployCodeCannotAuthorizeUntrustedChildren() public {
        EnvFactory localFactory = new EnvFactory();
        for (uint256 mode; mode < 2; mode++) {
            (bool created, bytes memory result) = invoke(
                mode == 1, FACTORY, abi.encodeCall(EnvFactory.deployCodeThrough, (localFactory))
            );
            require(created, "nested generic factory creation failed");
            address child = abi.decode(result, (address));
            (bool ok, bytes memory output) = child.call(abi.encodeCall(EnvReader.read, ()));
            expectDenied(ok, output, child);
            require(readDefault() == 7, "parent access lost after nested generic factory");
        }
    }

    function testForkDeployCodeRetainsAccessAfterNestedExecution() public {
        for (uint256 mode; mode < 3; mode++) {
            try vm.deployCode{gas: 5_000_000}("CheatcodeAccess.t.sol:RecursiveConstructorReader", abi.encode(mode)) returns (address child) {
                require(mode == 0, "nested constructor did not fail");
                require(EnvReader(child).read() == 7, "nested child runtime access lost");
            } catch {
                require(mode != 0, "nested child creation failed");
            }
            require(readDefault() == 7, "parent access lost after nested execution");
            (bool ok, bytes memory output) = READER.delegatecall(abi.encodeCall(EnvReader.read, ()));
            expectDenied(ok, output, READER);
        }
    }
}

contract CheatcodeAccessSelectForkTest is AccessFixture {
    function testForkAccessCheckedAfterSelectingFork() public {
        vm.createSelectFork(vm.envString("FOUNDRY_ACCESS_REGRESSION_FORK_URL"));
        vm.etch(READER, type(EnvReader).runtimeCode);
        (bool ok, bytes memory output) = READER.delegatecall(abi.encodeCall(EnvReader.read, ()));
        expectDenied(ok, output, READER);
        require(readDefault() == 7, "parent access lost after selecting fork");
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

contract RootDelegatedFactoryAccess is UsesVm {
    EnvFactory internal factory;
    bytes32[2] internal expectedErrors;

    function testForkRootDelegationCannotAuthorizeFactoryChildren() public {
        for (uint256 mode; mode < 2; mode++) {
            address child = factory.create(mode == 1, false, bytes32(mode));
            (bool ok, bytes memory output) = child.call(abi.encodeCall(EnvReader.read, ()));
            require(!ok, "delegated root authorized factory child");
            require(keccak256(output) == expectedErrors[mode], "unexpected authorization error");
        }
    }
}

contract CheatcodeAccessRootDelegationTest is RootDelegatedFactoryAccess {
    function setUp() public {
        factory = new EnvFactory();
        address[2] memory children = [
            vm.computeCreateAddress(address(factory), vm.getNonce(address(factory))),
            vm.computeCreate2Address(bytes32(uint256(1)), keccak256(type(EnvReader).creationCode), address(factory))
        ];
        for (uint256 mode; mode < children.length; mode++) {
            expectedErrors[mode] = denialHash(children[mode]);
        }
        address implementation = address(0x1234);
        vm.etch(implementation, type(RootDelegatedFactoryAccess).runtimeCode);
        vm.etch(address(this), abi.encodePacked(hex"ef0100", implementation));
    }
}
"#;
