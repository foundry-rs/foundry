// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

/// @notice Helper contract with a constructo that makes a call to itself then
///         optionally reverts if zero-length data is passed
contract SelfCaller {
    constructor(bytes memory) payable {
        assembly {
            // call self to test that the cheatcode correctly reports the
            // account as initialized even when there is no code at the
            // contract address
            pop(call(gas(), address(), div(callvalue(), 10), 0, 0, 0, 0))
            if eq(calldataload(0x04), 1) { revert(0, 0) }
        }
    }
}

/// @notice Helper contract with a constructor that stores a value in storage
///         and then optionally reverts.
contract ConstructorStorer {
    constructor(bool shouldRevert) {
        assembly {
            sstore(0x00, 0x01)
            if shouldRevert { revert(0, 0) }
        }
    }
}

/// @notice Helper contract that calls itself from the run method
contract Doer {
    uint256[10] spacer;
    mapping(bytes32 key => uint256 value) slots;

    function run() public payable {
        slots[bytes32("doer 1")]++;
        this.doStuff{value: msg.value / 10}();
    }

    function doStuff() external payable {
        slots[bytes32("doer 2")]++;
    }
}

/// @notice Helper contract that selfdestructs to a target address within its
///         constructor
contract SelfDestructor {
    constructor(address target) payable {
        selfdestruct(payable(target));
    }
}

/// @notice Helper contract that calls a Doer from the run method
contract Create2or {
    function create2(bytes32 salt, bytes memory initcode) external payable returns (address result) {
        assembly {
            result := create2(callvalue(), add(initcode, 0x20), mload(initcode), salt)
        }
    }
}

/// @notice Helper contract that calls a Doer from the run method and then
///         reverts
contract Reverter {
    Doer immutable doer;
    mapping(bytes32 key => uint256 value) slots;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public payable {
        slots[bytes32("reverter")]++;
        doer.run{value: msg.value / 10}();
        revert();
    }
}

/// @notice Helper contract that calls a Doer from the run method
contract Succeeder {
    Doer immutable doer;
    mapping(bytes32 key => uint256 value) slots;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public payable {
        slots[bytes32("succeeder")]++;
        doer.run{value: msg.value / 10}();
    }
}

/// @notice Helper contract that calls a Reverter and Succeeder from the run
///         method
contract NestedRunner {
    Doer public immutable doer;
    Reverter public immutable reverter;
    Succeeder public immutable succeeder;
    mapping(bytes32 key => uint256 value) slots;

    constructor() {
        doer = new Doer();
        reverter = new Reverter(doer);
        succeeder = new Succeeder(doer);
    }

    function run(bool shouldRevert) public payable {
        slots[bytes32("runner")]++;
        try reverter.run{value: msg.value / 10}() {
            if (shouldRevert) {
                revert();
            }
        } catch {}
        succeeder.run{value: msg.value / 10}();
        if (shouldRevert) {
            revert();
        }
    }
}

/// @notice Helper contract that directly reads from and writes to storage
contract StorageAccessor {
    function read(bytes32 slot) public view returns (bytes32 value) {
        assembly {
            value := sload(slot)
        }
    }

    function write(bytes32 slot, bytes32 value) public {
        assembly {
            sstore(slot, value)
        }
    }
}

/// @notice Test that the cheatcode correctly records account accesses
contract RecordAccountAccessesTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);
    NestedRunner runner;
    Create2or create2or;
    StorageAccessor test1;
    StorageAccessor test2;

    function setUp() public {
        runner = new NestedRunner();
        create2or = new Create2or();
        test1 = new StorageAccessor();
        test2 = new StorageAccessor();
    }

    /// @notice Test normal, non-nested storage accesses
    function testStorageAccesses() public {
        StorageAccessor one = test1;
        StorageAccessor two = test2;
        cheats.recordStateDiff();

        one.read(bytes32(uint256(1234)));
        one.write(bytes32(uint256(1235)), bytes32(uint256(5678)));
        two.write(bytes32(uint256(5678)), bytes32(uint256(123469)));
        two.write(bytes32(uint256(5678)), bytes32(uint256(1234)));

        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        // Empty since no record account is associated with the storage access
        assertEq(called.length, 4, "incorrect length");

        assertEq(called[0].storageAccesses.length, 1, "incorrect length");
        Vm.StorageAccess memory access = called[0].storageAccesses[0];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(one),
                slot: bytes32(uint256(1234)),
                isWrite: false,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(0)),
                reverted: false
            })
        );

        assertEq(called[1].storageAccesses.length, 1, "incorrect length");
        access = called[1].storageAccesses[0];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(one),
                slot: bytes32(uint256(1235)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(5678)),
                reverted: false
            })
        );

        assertEq(called[2].storageAccesses.length, 1, "incorrect length");
        access = called[2].storageAccesses[0];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(two),
                slot: bytes32(uint256(5678)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(123469)),
                reverted: false
            })
        );

        assertEq(called[3].storageAccesses.length, 1, "incorrect length");
        access = called[3].storageAccesses[0];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(two),
                slot: bytes32(uint256(5678)),
                isWrite: true,
                previousValue: bytes32(uint256(123469)),
                newValue: bytes32(uint256(1234)),
                reverted: false
            })
        );
    }

    /// @notice Test that basic account accesses are correctly recorded
    function testRecordAccountAccesses() public {
        cheats.recordStateDiff();

        (bool succ,) = address(1234).call("");
        (succ,) = address(5678).call{value: 1 ether}("");
        (succ,) = address(123469).call("hello world");
        (succ,) = address(5678).call("");
        // contract calls to self in constructor
        SelfCaller caller = new SelfCaller{value: 2 ether}('hello2 world2');

        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        assertEq(called.length, 6);
        assertEq(
            called[0],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(1234),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: hex"",
                value: 0,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );

        assertEq(
            called[1],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(5678),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                oldBalance: 0,
                newBalance: 1 ether,
                deployedCode: hex"",
                value: 1 ether,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(123469),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: hex"",
                value: 0,
                data: "hello world",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[3],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(5678),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                oldBalance: 1 ether,
                newBalance: 1 ether,
                deployedCode: hex"",
                value: 0,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[4],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(caller),
                kind: Vm.AccountAccessKind.Create,
                initialized: true,
                oldBalance: 0,
                newBalance: 2 ether,
                deployedCode: address(caller).code,
                value: 2 ether,
                data: abi.encodePacked(type(SelfCaller).creationCode, abi.encode("hello2 world2")),
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[5],
            Vm.AccountAccess({
                accessor: address(caller),
                account: address(caller),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                oldBalance: 2 ether,
                newBalance: 2 ether,
                deployedCode: hex"",
                value: 0.2 ether,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
    }

    /// @notice Test that account accesses are correctly recorded when a call
    ///         reverts
    function testRevertingCall() public {
        uint256 initBalance = address(this).balance;
        cheats.recordStateDiff();
        try this.revertingCall{value: 1 ether}(address(1234), "") {} catch {}
        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        assertEq(called.length, 2);
        assertEq(
            called[0],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(this),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                oldBalance: initBalance,
                newBalance: initBalance,
                deployedCode: hex"",
                value: 1 ether,
                data: abi.encodeCall(this.revertingCall, (address(1234), "")),
                reverted: true,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[1],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(1234),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                oldBalance: 0,
                newBalance: 0.1 ether,
                deployedCode: hex"",
                value: 0.1 ether,
                data: "",
                reverted: true,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
    }

    /// @notice Test that nested account accesses are correctly recorded
    function testNested() public {
        cheats.recordStateDiff();
        runNested(false, false);
    }

    /// @notice Test that nested account accesses are correctly recorded when
    ///         the first call reverts
    function testNested_Revert() public {
        cheats.recordStateDiff();
        runNested(true, false);
    }

    /// @notice Helper function to test nested account accesses
    /// @param shouldRevert Whether the first call should revert
    function runNested(bool shouldRevert, bool expectFirstCall) public {
        try runner.run{value: 1 ether}(shouldRevert) {} catch {}
        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        assertEq(called.length, 7 + toUint(expectFirstCall), "incorrect length");

        uint256 startingIndex = toUint(expectFirstCall);
        if (expectFirstCall) {
            assertEq(
                called[0],
                Vm.AccountAccess({
                    accessor: address(this),
                    account: address(1234),
                    kind: Vm.AccountAccessKind.Call,
                    oldBalance: 0,
                    newBalance: 0,
                    deployedCode: "",
                    initialized: false,
                    value: 0,
                    data: "",
                    reverted: false,
                    storageAccesses: new Vm.StorageAccess[](0)
                })
            );
        }

        assertEq(called[startingIndex].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex].storageAccesses[0],
            called[startingIndex].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner),
                slot: keccak256(abi.encodePacked(bytes32("runner"), bytes32(0))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(runner),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0,
                newBalance: shouldRevert ? 0 : 0.9 ether,
                deployedCode: "",
                initialized: true,
                value: 1 ether,
                data: abi.encodeCall(NestedRunner.run, (shouldRevert)),
                reverted: shouldRevert,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );

        assertEq(called[startingIndex + 1].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex + 1].storageAccesses[0],
            called[startingIndex + 1].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner.reverter()),
                slot: keccak256(abi.encodePacked(bytes32("reverter"), bytes32(0))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );
        assertEq(
            called[startingIndex + 1],
            Vm.AccountAccess({
                accessor: address(runner),
                account: address(runner.reverter()),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: "",
                initialized: true,
                value: 0.1 ether,
                data: abi.encodeCall(Reverter.run, ()),
                reverted: true,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );

        assertEq(called[startingIndex + 2].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex + 2].storageAccesses[0],
            called[startingIndex + 2].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 1"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );
        assertEq(
            called[startingIndex + 2],
            Vm.AccountAccess({
                accessor: address(runner.reverter()),
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0,
                newBalance: 0.01 ether,
                deployedCode: "",
                initialized: true,
                value: 0.01 ether,
                data: abi.encodeCall(Doer.run, ()),
                reverted: true,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );

        assertEq(called[startingIndex + 3].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex + 3].storageAccesses[0],
            called[startingIndex + 3].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 2"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );
        assertEq(
            called[startingIndex + 3],
            Vm.AccountAccess({
                accessor: address(runner.doer()),
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0.01 ether,
                newBalance: 0.01 ether,
                deployedCode: "",
                initialized: true,
                value: 0.001 ether,
                data: abi.encodeCall(Doer.doStuff, ()),
                reverted: true,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );

        assertEq(called[startingIndex + 4].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex + 4].storageAccesses[0],
            called[startingIndex + 4].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner.succeeder()),
                slot: keccak256(abi.encodePacked(bytes32("succeeder"), uint256(0))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex + 4],
            Vm.AccountAccess({
                accessor: address(runner),
                account: address(runner.succeeder()),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0,
                newBalance: 0.09 ether,
                deployedCode: "",
                initialized: true,
                value: 0.1 ether,
                data: abi.encodeCall(Succeeder.run, ()),
                reverted: shouldRevert,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );

        assertEq(called[startingIndex + 5].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex + 5].storageAccesses[0],
            called[startingIndex + 5].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 1"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex + 5],
            Vm.AccountAccess({
                accessor: address(runner.succeeder()),
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0,
                newBalance: 0.01 ether,
                deployedCode: "",
                initialized: true,
                value: 0.01 ether,
                data: abi.encodeCall(Doer.run, ()),
                reverted: shouldRevert,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );

        assertEq(called[startingIndex + 3].storageAccesses.length, 2, "incorrect length");
        assertIncrementEq(
            called[startingIndex + 6].storageAccesses[0],
            called[startingIndex + 6].storageAccesses[1],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 2"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex + 6],
            Vm.AccountAccess({
                accessor: address(runner.doer()),
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0.01 ether,
                newBalance: 0.01 ether,
                deployedCode: "",
                initialized: true,
                value: 0.001 ether,
                data: abi.encodeCall(Doer.doStuff, ()),
                reverted: shouldRevert,
                storageAccesses: new Vm.StorageAccess[](0)
            }),
            false
        );
    }

    /// @notice Test that constructor account and storage accesses are recorded, including reverts
    function testConstructorStorage() public {
        cheats.recordStateDiff();
        address storer = address(new ConstructorStorer(false));
        try create2or.create2(bytes32(0), abi.encodePacked(type(ConstructorStorer).creationCode, abi.encode(true))) {}
            catch {}
        bytes memory creationCode = abi.encodePacked(type(ConstructorStorer).creationCode, abi.encode(true));
        address hypotheticalStorer = deriveCreate2Address(address(create2or), bytes32(0), keccak256(creationCode));

        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        assertEq(called.length, 3, "incorrect account access length");
        assertEq(toUint(called[0].kind), toUint(Vm.AccountAccessKind.Create), "incorrect kind");
        assertEq(toUint(called[1].kind), toUint(Vm.AccountAccessKind.Call), "incorrect kind");
        assertEq(toUint(called[2].kind), toUint(Vm.AccountAccessKind.Create), "incorrect kind");

        Vm.StorageAccess[] memory storageAccesses = new Vm.StorageAccess[](1);

        assertEq(called[0].storageAccesses.length, 1, "incorrect storage access length");
        storageAccesses[0] = Vm.StorageAccess({
            account: storer,
            slot: bytes32(uint256(0)),
            isWrite: true,
            previousValue: bytes32(uint256(0)),
            newValue: bytes32(uint256(1)),
            reverted: false
        });
        assertEq(
            called[0],
            Vm.AccountAccess({
                accessor: address(this),
                account: address(storer),
                kind: Vm.AccountAccessKind.Create,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: storer.code,
                initialized: true,
                value: 0,
                data: abi.encodePacked(type(ConstructorStorer).creationCode, abi.encode(false)),
                reverted: false,
                storageAccesses: storageAccesses
            })
        );

        assertEq(called[1].storageAccesses.length, 0, "incorrect storage access length");

        assertEq(called[2].storageAccesses.length, 1, "incorrect storage access length");
        assertEq(
            called[2].storageAccesses[0],
            Vm.StorageAccess({
                account: hypotheticalStorer,
                slot: bytes32(uint256(0)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );
    }

    /// @notice Test that account accesses are correctly recorded when the
    ///         recording is started from a lower depth than they are
    ///         retrieved
    function testNested_LowerDepth() public {
        this.startRecordingFromLowerDepth();
        runNested(false, true);
    }

    /// @notice Test that account accesses are correctly recorded when
    ///         the first call reverts the and recording is started from
    ///         a lower depth than they are retrieved.
    function testNested_LowerDepth_Revert() public {
        this.startRecordingFromLowerDepth();
        runNested(true, true);
    }

    /// @notice Test that constructor calls and calls made within a constructor
    ///         are correctly recorded, even if it reverts
    function testCreateRevert() public {
        cheats.recordStateDiff();
        bytes memory creationCode = abi.encodePacked(type(SelfCaller).creationCode, abi.encode(""));
        try create2or.create2(bytes32(0), creationCode) {} catch {}
        address hypotheticalAddress = deriveCreate2Address(address(create2or), bytes32(0), keccak256(creationCode));
        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        assertEq(called.length, 3, "incorrect length");
        assertEq(
            called[1],
            Vm.AccountAccess({
                accessor: address(create2or),
                account: hypotheticalAddress,
                kind: Vm.AccountAccessKind.Create,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: address(hypotheticalAddress).code,
                initialized: true,
                value: 0,
                data: creationCode,
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                accessor: hypotheticalAddress,
                account: hypotheticalAddress,
                kind: Vm.AccountAccessKind.Call,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: hex"",
                initialized: true,
                value: 0,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
    }

    /// @notice It is important to test SELFDESTRUCT behavior as long as there
    ///         are public networks that support the opcode, regardless of whether
    ///         or not Ethereum mainnet does.
    function testSelfDestruct() public {
        uint256 startingBalance = address(this).balance;
        this.startRecordingFromLowerDepth();
        address a = address(new SelfDestructor{value:1 ether}(address(this)));
        address b = address(new SelfDestructor{value:1 ether}(address(bytes20("doesn't exist yet"))));
        Vm.AccountAccess[] memory called = cheats.getStateDiff();
        assertEq(called.length, 5, "incorrect length");
        assertEq(
            called[1],
            Vm.AccountAccess({
                accessor: address(this),
                account: a,
                kind: Vm.AccountAccessKind.Create,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: "",
                initialized: true,
                value: 1 ether,
                data: abi.encodePacked(type(SelfDestructor).creationCode, abi.encode(address(this))),
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                accessor: address(a),
                account: address(this),
                kind: Vm.AccountAccessKind.SelfDestruct,
                oldBalance: startingBalance - 1 ether,
                newBalance: startingBalance,
                deployedCode: "",
                initialized: true,
                value: 1 ether,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[3],
            Vm.AccountAccess({
                accessor: address(this),
                account: b,
                kind: Vm.AccountAccessKind.Create,
                oldBalance: 0,
                newBalance: 0,
                deployedCode: "",
                initialized: true,
                value: 1 ether,
                data: abi.encodePacked(type(SelfDestructor).creationCode, abi.encode(address(bytes20("doesn't exist yet")))),
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
        assertEq(
            called[4],
            Vm.AccountAccess({
                accessor: address(b),
                account: address(bytes20("doesn't exist yet")),
                kind: Vm.AccountAccessKind.SelfDestruct,
                oldBalance: 0,
                newBalance: 1 ether,
                deployedCode: hex"",
                initialized: false,
                value: 1 ether,
                data: "",
                reverted: false,
                storageAccesses: new Vm.StorageAccess[](0)
            })
        );
    }

    function startRecordingFromLowerDepth() external {
        cheats.recordStateDiff();
        assembly {
            pop(call(gas(), 1234, 0, 0, 0, 0, 0))
        }
    }

    function revertingCall(address target, bytes memory data) external payable {
        assembly {
            pop(call(gas(), target, div(callvalue(), 10), add(data, 0x20), mload(data), 0, 0))
        }
        revert();
    }

    function assertIncrementEq(
        Vm.StorageAccess memory read,
        Vm.StorageAccess memory write,
        Vm.StorageAccess memory expected
    ) internal {
        assertEq(
            read,
            Vm.StorageAccess({
                account: expected.account,
                slot: expected.slot,
                isWrite: false,
                previousValue: expected.previousValue,
                newValue: expected.previousValue,
                reverted: expected.reverted
            })
        );
        assertEq(
            write,
            Vm.StorageAccess({
                account: expected.account,
                slot: expected.slot,
                isWrite: true,
                previousValue: expected.previousValue,
                newValue: expected.newValue,
                reverted: expected.reverted
            })
        );
    }

    function assertEq(Vm.AccountAccess memory actualAccess, Vm.AccountAccess memory expectedAccess) internal {
        assertEq(actualAccess, expectedAccess, true);
    }

    function assertEq(Vm.AccountAccess memory actualAccess, Vm.AccountAccess memory expectedAccess, bool checkStorage)
        internal
    {
        assertEq(actualAccess.accessor, expectedAccess.accessor, "incorrect accessor");
        assertEq(actualAccess.account, expectedAccess.account, "incorrect account");
        assertEq(toUint(actualAccess.kind), toUint(expectedAccess.kind), "incorrect kind");
        assertEq(toUint(actualAccess.initialized), toUint(expectedAccess.initialized), "incorrect initialized");
        assertEq(actualAccess.oldBalance, expectedAccess.oldBalance, "incorrect oldBalance");
        assertEq(actualAccess.newBalance, expectedAccess.newBalance, "incorrect newBalance");
        assertEq(actualAccess.deployedCode, expectedAccess.deployedCode, "incorrect deployedCode");
        assertEq(actualAccess.value, expectedAccess.value, "incorrect value");
        assertEq(actualAccess.data, expectedAccess.data, "incorrect data");
        assertEq(toUint(actualAccess.reverted), toUint(expectedAccess.reverted), "incorrect reverted");
        if (checkStorage) {
            assertEq(
                actualAccess.storageAccesses.length,
                expectedAccess.storageAccesses.length,
                "incorrect storageAccesses length"
            );
            for (uint256 i = 0; i < actualAccess.storageAccesses.length; i++) {
                assertEq(actualAccess.storageAccesses[i], expectedAccess.storageAccesses[i]);
            }
        }
    }

    function assertEq(Vm.StorageAccess memory actual, Vm.StorageAccess memory expected) internal {
        assertEq(actual.account, expected.account, "incorrect storageAccess account");
        assertEq(actual.slot, expected.slot, "incorrect storageAccess slot");
        assertEq(toUint(actual.isWrite), toUint(expected.isWrite), "incorrect storageAccess isWrite");
        assertEq(actual.previousValue, expected.previousValue, "incorrect storageAccess previousValue");
        assertEq(actual.newValue, expected.newValue, "incorrect storageAccess newValue");
        assertEq(toUint(actual.reverted), toUint(expected.reverted), "incorrect storageAccess reverted");
    }

    function toUint(Vm.AccountAccessKind kind) internal pure returns (uint256 value) {
        assembly {
            value := and(kind, 0xff)
        }
    }

    function toUint(bool a) internal pure returns (uint256) {
        return a ? 1 : 0;
    }

    function deriveCreate2Address(address deployer, bytes32 salt, bytes32 codeHash) internal pure returns (address) {
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), deployer, salt, codeHash)))));
    }
}
