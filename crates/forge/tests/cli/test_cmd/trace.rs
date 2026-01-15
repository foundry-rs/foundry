//! Tests for tracing functionality

use foundry_test_utils::str;

forgetest_init!(conflicting_signatures, |prj, cmd| {
    prj.add_test(
        "ConflictingSignatures.t.sol",
        r#"
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

contract ReturnsNothing {
    function func() public pure {}
}

contract ReturnsString {
    function func() public pure returns (string memory) {
        return "string";
    }
}

contract ReturnsUint {
    function func() public pure returns (uint256) {
        return 1;
    }
}

contract ConflictingSignaturesTest is Test {
    ReturnsNothing retsNothing;
    ReturnsString retsString;
    ReturnsUint retsUint;

    function setUp() public {
        retsNothing = new ReturnsNothing();
        retsString = new ReturnsString();
        retsUint = new ReturnsUint();
    }

    /// Tests that traces are decoded properly when multiple
    /// functions have the same 4byte signature, but different
    /// return values.
    function testTraceWithConflictingSignatures() public {
        retsNothing.func();
        retsString.func();
        retsUint.func();
    }
}
"#,
    );

    cmd.args(["test", "-vvvvv"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/ConflictingSignatures.t.sol:ConflictingSignaturesTest
[PASS] testTraceWithConflictingSignatures() ([GAS])
Traces:
  [..] ConflictingSignaturesTest::setUp()
    ├─ [..] → new ReturnsNothing@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 106 bytes of code
    ├─ [..] → new ReturnsString@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   └─ ← [Return] 334 bytes of code
    ├─ [..] → new ReturnsUint@0xF62849F9A0B5Bf2913b396098F7c7019b51A820a
    │   └─ ← [Return] 175 bytes of code
    └─ ← [Stop]

  [..] ConflictingSignaturesTest::testTraceWithConflictingSignatures()
    ├─ [..] ReturnsNothing::func() [staticcall]
    │   └─ ← [Stop]
    ├─ [..] ReturnsString::func() [staticcall]
    │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000006737472696e670000000000000000000000000000000000000000000000000000
    ├─ [..] ReturnsUint::func() [staticcall]
    │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000001
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(trace_test, |prj, cmd| {
    prj.add_test(
        "Trace.t.sol",
        r#"
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

contract RecursiveCall {
    TraceTest factory;

    event Depth(uint256 depth);
    event ChildDepth(uint256 childDepth);
    event CreatedChild(uint256 childDepth);

    constructor(address _factory) {
        factory = TraceTest(_factory);
    }

    function recurseCall(uint256 neededDepth, uint256 depth) public returns (uint256) {
        if (depth == neededDepth) {
            this.negativeNum();
            return neededDepth;
        }

        uint256 childDepth = this.recurseCall(neededDepth, depth + 1);
        emit ChildDepth(childDepth);

        this.someCall();
        emit Depth(depth);

        return depth;
    }

    function recurseCreate(uint256 neededDepth, uint256 depth) public returns (uint256) {
        if (depth == neededDepth) {
            return neededDepth;
        }

        RecursiveCall child = factory.create();
        emit CreatedChild(depth + 1);

        uint256 childDepth = child.recurseCreate(neededDepth, depth + 1);
        emit ChildDepth(childDepth);
        emit Depth(depth);

        return depth;
    }

    function someCall() public pure {}

    function negativeNum() public pure returns (int256) {
        return -1000000000;
    }
}

contract TraceTest is Test {
    uint256 nodeId = 0;
    RecursiveCall first;

    function setUp() public {
        first = this.create();
    }

    function create() public returns (RecursiveCall) {
        RecursiveCall node = new RecursiveCall(address(this));
        vm.label(address(node), string(abi.encodePacked("Node ", uintToString(nodeId++))));

        return node;
    }

    function testRecurseCall() public {
        first.recurseCall(8, 0);
    }

    function testRecurseCreate() public {
        first.recurseCreate(8, 0);
    }
}

function uintToString(uint256 value) pure returns (string memory) {
    // Taken from OpenZeppelin
    if (value == 0) {
        return "0";
    }
    uint256 temp = value;
    uint256 digits;
    while (temp != 0) {
        digits++;
        temp /= 10;
    }
    bytes memory buffer = new bytes(digits);
    while (value != 0) {
        digits -= 1;
        buffer[digits] = bytes1(uint8(48 + uint256(value % 10)));
        value /= 10;
    }
    return string(buffer);
}
"#,
    );

    cmd.args(["test", "-vvvvv"]).assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/Trace.t.sol:TraceTest
[PASS] testRecurseCall() ([GAS])
Traces:
  [..] TraceTest::setUp()
    ├─ [..] TraceTest::create()
    │   ├─ [..] → new Node 0@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   │   └─ ← [Return] 1911 bytes of code
    │   ├─ [0] VM::label(Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f], "Node 0")
    │   │   └─ ← [Return]
    │   └─ ← [Return] Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f]
    └─ ← [Stop]

  [..] TraceTest::testRecurseCall()
    ├─ [..] Node 0::recurseCall(8, 0)
    │   ├─ [..] Node 0::recurseCall(8, 1)
    │   │   ├─ [..] Node 0::recurseCall(8, 2)
    │   │   │   ├─ [..] Node 0::recurseCall(8, 3)
    │   │   │   │   ├─ [..] Node 0::recurseCall(8, 4)
    │   │   │   │   │   ├─ [..] Node 0::recurseCall(8, 5)
    │   │   │   │   │   │   ├─ [..] Node 0::recurseCall(8, 6)
    │   │   │   │   │   │   │   ├─ [..] Node 0::recurseCall(8, 7)
    │   │   │   │   │   │   │   │   ├─ [..] Node 0::recurseCall(8, 8)
    │   │   │   │   │   │   │   │   │   ├─ [..] Node 0::negativeNum() [staticcall]
    │   │   │   │   │   │   │   │   │   │   └─ ← [Return] -1000000000 [-1e9]
    │   │   │   │   │   │   │   │   │   └─ ← [Return] 8
    │   │   │   │   │   │   │   │   ├─ emit ChildDepth(childDepth: 8)
    │   │   │   │   │   │   │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   │   │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   │   │   │   │   ├─ emit Depth(depth: 7)
    │   │   │   │   │   │   │   │   └─ ← [Return] 7
    │   │   │   │   │   │   │   ├─ emit ChildDepth(childDepth: 7)
    │   │   │   │   │   │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   │   │   │   ├─ emit Depth(depth: 6)
    │   │   │   │   │   │   │   └─ ← [Return] 6
    │   │   │   │   │   │   ├─ emit ChildDepth(childDepth: 6)
    │   │   │   │   │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   │   │   ├─ emit Depth(depth: 5)
    │   │   │   │   │   │   └─ ← [Return] 5
    │   │   │   │   │   ├─ emit ChildDepth(childDepth: 5)
    │   │   │   │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   │   ├─ emit Depth(depth: 4)
    │   │   │   │   │   └─ ← [Return] 4
    │   │   │   │   ├─ emit ChildDepth(childDepth: 4)
    │   │   │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   ├─ emit Depth(depth: 3)
    │   │   │   │   └─ ← [Return] 3
    │   │   │   ├─ emit ChildDepth(childDepth: 3)
    │   │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   │   └─ ← [Stop]
    │   │   │   ├─ emit Depth(depth: 2)
    │   │   │   └─ ← [Return] 2
    │   │   ├─ emit ChildDepth(childDepth: 2)
    │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   └─ ← [Stop]
    │   │   ├─ emit Depth(depth: 1)
    │   │   └─ ← [Return] 1
    │   ├─ emit ChildDepth(childDepth: 1)
    │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   └─ ← [Stop]
    │   ├─ emit Depth(depth: 0)
    │   └─ ← [Return] 0
    └─ ← [Stop]

[PASS] testRecurseCreate() ([GAS])
Traces:
  [..] TraceTest::setUp()
    ├─ [..] TraceTest::create()
    │   ├─ [..] → new Node 0@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   │   └─ ← [Return] 1911 bytes of code
    │   ├─ [0] VM::label(Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f], "Node 0")
    │   │   └─ ← [Return]
    │   └─ ← [Return] Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f]
    └─ ← [Stop]

  [..] TraceTest::testRecurseCreate()
    ├─ [..] Node 0::recurseCreate(8, 0)
    │   ├─ [..] TraceTest::create()
    │   │   ├─ [..] → new Node 1@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   │   │   ├─  storage changes:
    │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   ├─ [0] VM::label(Node 1: [0x2e234DAe75C793f67A35089C9d99245E1C58470b], "Node 1")
    │   │   │   └─ ← [Return]
    │   │   ├─  storage changes:
    │   │   │   @ 32: 1 → 2
    │   │   └─ ← [Return] Node 1: [0x2e234DAe75C793f67A35089C9d99245E1C58470b]
    │   ├─ emit CreatedChild(childDepth: 1)
    │   ├─ [..] Node 1::recurseCreate(8, 1)
    │   │   ├─ [..] TraceTest::create()
    │   │   │   ├─ [..] → new Node 2@0xF62849F9A0B5Bf2913b396098F7c7019b51A820a
    │   │   │   │   ├─  storage changes:
    │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   ├─ [0] VM::label(Node 2: [0xF62849F9A0B5Bf2913b396098F7c7019b51A820a], "Node 2")
    │   │   │   │   └─ ← [Return]
    │   │   │   ├─  storage changes:
    │   │   │   │   @ 32: 2 → 3
    │   │   │   └─ ← [Return] Node 2: [0xF62849F9A0B5Bf2913b396098F7c7019b51A820a]
    │   │   ├─ emit CreatedChild(childDepth: 2)
    │   │   ├─ [..] Node 2::recurseCreate(8, 2)
    │   │   │   ├─ [..] TraceTest::create()
    │   │   │   │   ├─ [..] → new Node 3@0x5991A2dF15A8F6A256D3Ec51E99254Cd3fb576A9
    │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   │   ├─ [0] VM::label(Node 3: [0x5991A2dF15A8F6A256D3Ec51E99254Cd3fb576A9], "Node 3")
    │   │   │   │   │   └─ ← [Return]
    │   │   │   │   ├─  storage changes:
    │   │   │   │   │   @ 32: 3 → 4
    │   │   │   │   └─ ← [Return] Node 3: [0x5991A2dF15A8F6A256D3Ec51E99254Cd3fb576A9]
    │   │   │   ├─ emit CreatedChild(childDepth: 3)
    │   │   │   ├─ [..] Node 3::recurseCreate(8, 3)
    │   │   │   │   ├─ [..] TraceTest::create()
    │   │   │   │   │   ├─ [..] → new Node 4@0xc7183455a4C133Ae270771860664b6B7ec320bB1
    │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   │   │   ├─ [0] VM::label(Node 4: [0xc7183455a4C133Ae270771860664b6B7ec320bB1], "Node 4")
    │   │   │   │   │   │   └─ ← [Return]
    │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   @ 32: 4 → 5
    │   │   │   │   │   └─ ← [Return] Node 4: [0xc7183455a4C133Ae270771860664b6B7ec320bB1]
    │   │   │   │   ├─ emit CreatedChild(childDepth: 4)
    │   │   │   │   ├─ [..] Node 4::recurseCreate(8, 4)
    │   │   │   │   │   ├─ [..] TraceTest::create()
    │   │   │   │   │   │   ├─ [..] → new Node 5@0xa0Cb889707d426A7A386870A03bc70d1b0697598
    │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   │   │   │   ├─ [0] VM::label(Node 5: [0xa0Cb889707d426A7A386870A03bc70d1b0697598], "Node 5")
    │   │   │   │   │   │   │   └─ ← [Return]
    │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   @ 32: 5 → 6
    │   │   │   │   │   │   └─ ← [Return] Node 5: [0xa0Cb889707d426A7A386870A03bc70d1b0697598]
    │   │   │   │   │   ├─ emit CreatedChild(childDepth: 5)
    │   │   │   │   │   ├─ [..] Node 5::recurseCreate(8, 5)
    │   │   │   │   │   │   ├─ [..] TraceTest::create()
    │   │   │   │   │   │   │   ├─ [..] → new Node 6@0x1d1499e622D69689cdf9004d05Ec547d650Ff211
    │   │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   │   │   │   │   ├─ [0] VM::label(Node 6: [0x1d1499e622D69689cdf9004d05Ec547d650Ff211], "Node 6")
    │   │   │   │   │   │   │   │   └─ ← [Return]
    │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   @ 32: 6 → 7
    │   │   │   │   │   │   │   └─ ← [Return] Node 6: [0x1d1499e622D69689cdf9004d05Ec547d650Ff211]
    │   │   │   │   │   │   ├─ emit CreatedChild(childDepth: 6)
    │   │   │   │   │   │   ├─ [..] Node 6::recurseCreate(8, 6)
    │   │   │   │   │   │   │   ├─ [..] TraceTest::create()
    │   │   │   │   │   │   │   │   ├─ [..] → new Node 7@0xA4AD4f68d0b91CFD19687c881e50f3A00242828c
    │   │   │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   │   │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   │   │   │   │   │   ├─ [0] VM::label(Node 7: [0xA4AD4f68d0b91CFD19687c881e50f3A00242828c], "Node 7")
    │   │   │   │   │   │   │   │   │   └─ ← [Return]
    │   │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   │   @ 32: 7 → 8
    │   │   │   │   │   │   │   │   └─ ← [Return] Node 7: [0xA4AD4f68d0b91CFD19687c881e50f3A00242828c]
    │   │   │   │   │   │   │   ├─ emit CreatedChild(childDepth: 7)
    │   │   │   │   │   │   │   ├─ [..] Node 7::recurseCreate(8, 7)
    │   │   │   │   │   │   │   │   ├─ [..] TraceTest::create()
    │   │   │   │   │   │   │   │   │   ├─ [..] → new Node 8@0x03A6a84cD762D9707A21605b548aaaB891562aAb
    │   │   │   │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   │   │   │   │   │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   │   │   │   │   │   │   │   ├─ [0] VM::label(Node 8: [0x03A6a84cD762D9707A21605b548aaaB891562aAb], "Node 8")
    │   │   │   │   │   │   │   │   │   │   └─ ← [Return]
    │   │   │   │   │   │   │   │   │   ├─  storage changes:
    │   │   │   │   │   │   │   │   │   │   @ 32: 8 → 9
    │   │   │   │   │   │   │   │   │   └─ ← [Return] Node 8: [0x03A6a84cD762D9707A21605b548aaaB891562aAb]
    │   │   │   │   │   │   │   │   ├─ emit CreatedChild(childDepth: 8)
    │   │   │   │   │   │   │   │   ├─ [..] Node 8::recurseCreate(8, 8)
    │   │   │   │   │   │   │   │   │   └─ ← [Return] 8
    │   │   │   │   │   │   │   │   ├─ emit ChildDepth(childDepth: 8)
    │   │   │   │   │   │   │   │   ├─ emit Depth(depth: 7)
    │   │   │   │   │   │   │   │   └─ ← [Return] 7
    │   │   │   │   │   │   │   ├─ emit ChildDepth(childDepth: 7)
    │   │   │   │   │   │   │   ├─ emit Depth(depth: 6)
    │   │   │   │   │   │   │   └─ ← [Return] 6
    │   │   │   │   │   │   ├─ emit ChildDepth(childDepth: 6)
    │   │   │   │   │   │   ├─ emit Depth(depth: 5)
    │   │   │   │   │   │   └─ ← [Return] 5
    │   │   │   │   │   ├─ emit ChildDepth(childDepth: 5)
    │   │   │   │   │   ├─ emit Depth(depth: 4)
    │   │   │   │   │   └─ ← [Return] 4
    │   │   │   │   ├─ emit ChildDepth(childDepth: 4)
    │   │   │   │   ├─ emit Depth(depth: 3)
    │   │   │   │   └─ ← [Return] 3
    │   │   │   ├─ emit ChildDepth(childDepth: 3)
    │   │   │   ├─ emit Depth(depth: 2)
    │   │   │   └─ ← [Return] 2
    │   │   ├─ emit ChildDepth(childDepth: 2)
    │   │   ├─ emit Depth(depth: 1)
    │   │   └─ ← [Return] 1
    │   ├─ emit ChildDepth(childDepth: 1)
    │   ├─ emit Depth(depth: 0)
    │   └─ ← [Return] 0
    └─ ← [Stop]

Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(trace_test_detph, |prj, cmd| {
    prj.add_test(
        "Trace.t.sol",
        r#"
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

contract RecursiveCall {
    TraceTest factory;

    event Depth(uint256 depth);
    event ChildDepth(uint256 childDepth);
    event CreatedChild(uint256 childDepth);

    constructor(address _factory) {
        factory = TraceTest(_factory);
    }

    function recurseCall(uint256 neededDepth, uint256 depth) public returns (uint256) {
        if (depth == neededDepth) {
            this.negativeNum();
            return neededDepth;
        }

        uint256 childDepth = this.recurseCall(neededDepth, depth + 1);
        emit ChildDepth(childDepth);

        this.someCall();
        emit Depth(depth);

        return depth;
    }

    function recurseCreate(uint256 neededDepth, uint256 depth) public returns (uint256) {
        if (depth == neededDepth) {
            return neededDepth;
        }

        RecursiveCall child = factory.create();
        emit CreatedChild(depth + 1);

        uint256 childDepth = child.recurseCreate(neededDepth, depth + 1);
        emit ChildDepth(childDepth);
        emit Depth(depth);

        return depth;
    }

    function someCall() public pure {}

    function negativeNum() public pure returns (int256) {
        return -1000000000;
    }
}

contract TraceTest is Test {
    uint256 nodeId = 0;
    RecursiveCall first;

    function setUp() public {
        first = this.create();
    }

    function create() public returns (RecursiveCall) {
        RecursiveCall node = new RecursiveCall(address(this));
        vm.label(address(node), string(abi.encodePacked("Node ", uintToString(nodeId++))));

        return node;
    }

    function testRecurseCall() public {
        first.recurseCall(8, 0);
    }

    function testRecurseCreate() public {
        first.recurseCreate(8, 0);
    }
}

function uintToString(uint256 value) pure returns (string memory) {
    // Taken from OpenZeppelin
    if (value == 0) {
        return "0";
    }
    uint256 temp = value;
    uint256 digits;
    while (temp != 0) {
        digits++;
        temp /= 10;
    }
    bytes memory buffer = new bytes(digits);
    while (value != 0) {
        digits -= 1;
        buffer[digits] = bytes1(uint8(48 + uint256(value % 10)));
        value /= 10;
    }
    return string(buffer);
}
"#,
    );

    cmd.args(["test", "-vvvvv", "--trace-depth", "3"]).assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/Trace.t.sol:TraceTest
[PASS] testRecurseCall() ([GAS])
Traces:
  [..] TraceTest::setUp()
    ├─ [..] TraceTest::create()
    │   ├─ [..] → new Node 0@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   │   └─ ← [Return] 1911 bytes of code
    │   ├─ [0] VM::label(Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f], "Node 0")
    │   │   └─ ← [Return]
    │   └─ ← [Return] Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f]
    └─ ← [Stop]

  [..] TraceTest::testRecurseCall()
    ├─ [..] Node 0::recurseCall(8, 0)
    │   ├─ [..] Node 0::recurseCall(8, 1)
    │   │   ├─ [..] Node 0::recurseCall(8, 2)
    │   │   │   └─ ← [Return] 2
    │   │   ├─ emit ChildDepth(childDepth: 2)
    │   │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   │   └─ ← [Stop]
    │   │   ├─ emit Depth(depth: 1)
    │   │   └─ ← [Return] 1
    │   ├─ emit ChildDepth(childDepth: 1)
    │   ├─ [..] Node 0::someCall() [staticcall]
    │   │   └─ ← [Stop]
    │   ├─ emit Depth(depth: 0)
    │   └─ ← [Return] 0
    └─ ← [Stop]

[PASS] testRecurseCreate() ([GAS])
Traces:
  [..] TraceTest::setUp()
    ├─ [..] TraceTest::create()
    │   ├─ [..] → new Node 0@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   │   └─ ← [Return] 1911 bytes of code
    │   ├─ [0] VM::label(Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f], "Node 0")
    │   │   └─ ← [Return]
    │   └─ ← [Return] Node 0: [0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f]
    └─ ← [Stop]

  [..] TraceTest::testRecurseCreate()
    ├─ [..] Node 0::recurseCreate(8, 0)
    │   ├─ [..] TraceTest::create()
    │   │   ├─ [405132] → new Node 1@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   │   │   ├─  storage changes:
    │   │   │   │   @ 0: 0 → 0x0000000000000000000000007fa9385be102ac3eac297483dd6233d62b3e1496
    │   │   │   └─ ← [Return] 1911 bytes of code
    │   │   ├─ [0] VM::label(Node 1: [0x2e234DAe75C793f67A35089C9d99245E1C58470b], "Node 1")
    │   │   │   └─ ← [Return]
    │   │   ├─  storage changes:
    │   │   │   @ 32: 1 → 2
    │   │   └─ ← [Return] Node 1: [0x2e234DAe75C793f67A35089C9d99245E1C58470b]
    │   ├─ emit CreatedChild(childDepth: 1)
    │   ├─ [..] Node 1::recurseCreate(8, 1)
    │   │   ├─ [..] TraceTest::create()
    │   │   │   ├─  storage changes:
    │   │   │   │   @ 32: 2 → 3
    │   │   │   └─ ← [Return] Node 2: [0xF62849F9A0B5Bf2913b396098F7c7019b51A820a]
    │   │   ├─ emit CreatedChild(childDepth: 2)
    │   │   ├─ [..] Node 2::recurseCreate(8, 2)
    │   │   │   └─ ← [Return] 2
    │   │   ├─ emit ChildDepth(childDepth: 2)
    │   │   ├─ emit Depth(depth: 1)
    │   │   └─ ← [Return] 1
    │   ├─ emit ChildDepth(childDepth: 1)
    │   ├─ emit Depth(depth: 0)
    │   └─ ← [Return] 0
    └─ ← [Stop]

Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

// Test for issue #12962: trace should show receive() instead of fallback() for empty calldata
// when calling a contract deployed from raw bytecode.
// See: https://github.com/foundry-rs/foundry/issues/12962
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(receive_vs_fallback_trace, |prj, cmd| {
    prj.add_test(
        "ReceiveVsFallback.t.sol",
        r#"
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

contract ReceiveVsFallbackTest is Test {
    /// Test that deploying a contract from raw runtime bytecode and calling it with empty calldata
    /// correctly shows receive() in the trace, not fallback().
    /// This is a regression test for https://github.com/foundry-rs/foundry/issues/12962
    function testReceiveTraceForRawBytecode() public {
        // Raw runtime bytecode of a contract with receive() and fallback() that emit events.
        // The contract emits Log("receive", sender, value, data) for receive() calls
        // and Log("fallback", sender, value, data) for fallback() calls.
        bytes memory code = hex"608060405236610044577ff7f75251dee7d7fc22deac3247729ebe7c86541f35930bf10c2a4207479a3b6c333460405161003a929190610172565b60405180910390a1005b7ff7f75251dee7d7fc22deac3247729ebe7c86541f35930bf10c2a4207479a3b6c333460003660405161007a949392919061025a565b60405180910390a1005b600082825260208201905092915050565b7f7265636569766500000000000000000000000000000000000000000000000000600082015250565b60006100cb600783610084565b91506100d682610095565b602082019050919050565b600073ffffffffffffffffffffffffffffffffffffffff82169050919050565b600061010c826100e1565b9050919050565b61011c81610101565b82525050565b6000819050919050565b61013581610122565b82525050565b600082825260208201905092915050565b50565b600061015c60008361013b565b91506101678261014c565b600082019050919050565b6000608082019050818103600083015261018b816100be565b905061019a6020830185610113565b6101a7604083018461012c565b81810360608301526101b88161014f565b90509392505050565b7f66616c6c6261636b000000000000000000000000000000000000000000000000600082015250565b60006101f7600883610084565b9150610202826101c1565b602082019050919050565b82818337600083830152505050565b6000601f19601f8301169050919050565b6000610239838561013b565b935061024683858461020d565b61024f8361021c565b840190509392505050565b60006080820190508181036000830152610273816101ea565b90506102826020830187610113565b61028f604083018661012c565b81810360608301526102a281848661022d565b90509594505050505056fea2646970667358221220749a95417d16869c0020868fb950c3602810c6148809abb4489b45f51181536964736f6c63430008130033";

        // Prepend minimal constructor: PUSH2 size, PUSH1 0x0E, PUSH1 0x00, CODECOPY, PUSH2 size, PUSH1 0x00, RETURN
        bytes memory init = hex"6102e3600e6000396102e36000f3";
        bytes memory initcode = abi.encodePacked(init, code);

        address deployed;
        assembly {
            deployed := create(0, add(initcode, 0x20), mload(initcode))
        }
        require(deployed != address(0), "deployment failed");

        // Call with empty calldata - should invoke receive() and show "receive()" in trace
        (bool success,) = deployed.call("");
        assertTrue(success, "call should succeed");
    }
}
"#,
    );

    // Test receive() trace - should show "receive()" not "fallback()"
    cmd.args(["test", "--match-test", "testReceiveTraceForRawBytecode", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] testReceiveTraceForRawBytecode() ([GAS])
Traces:
  [..] ReceiveVsFallbackTest::testReceiveTraceForRawBytecode()
    ├─ [..] → new <unknown>@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 739 bytes of code
    ├─ [..] 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f::receive()
    │   ├─ emit Log(: "receive", : ReceiveVsFallbackTest: [0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496], : 0, : 0x)
    │   └─ ← [Stop]
    └─ ← [Stop]

...
"#]]);
});
