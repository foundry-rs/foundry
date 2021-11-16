// Taken from:
// https://github.com/dapphub/dapptools/blob/e41b6cd9119bbd494aba1236838b859f2136696b/src/dapp-tests/pass/cheatCodes.sol
pragma solidity ^0.6.6;
pragma experimental ABIEncoderV2;

import "./DsTest.sol";

interface Hevm {
    function warp(uint256) external;
    function roll(uint256) external;
    function load(address,bytes32) external returns (bytes32);
    function store(address,bytes32,bytes32) external;
    function sign(uint256,bytes32) external returns (uint8,bytes32,bytes32);
    function addr(uint256) external returns (address);
    function ffi(string[] calldata) external returns (bytes memory);
}

contract HasStorage {
    uint public slot0 = 10;
}

// We add `assertEq` tests as well to ensure that our test runner checks the
// `failed` variable.
contract CheatCodes is DSTest {
    address public store = address(new HasStorage());
    Hevm constant hevm = Hevm(HEVM_ADDRESS);

    // Warp

    function testWarp(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        require(block.timestamp == pre + jump, "warp failed");
    }

    function testWarpAssertEq(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        assertEq(block.timestamp, pre + jump);
    }

    function testFailWarp(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        require(block.timestamp == pre + jump + 1, "warp failed");
    }

    function testFailWarpAssert(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        assertEq(block.timestamp, pre + jump + 1);
    }

    // Roll

    // Underscore does not run the fuzz test?!
    function testRoll(uint256 jump) public {
        uint pre = block.number;
        hevm.roll(block.number + jump);
        require(block.number == pre + jump, "roll failed");
    }

    function testFailRoll(uint32 jump) public {
        uint pre = block.number;
        hevm.roll(block.number + jump);
        assertEq(block.number, pre + jump + 1);
    }

    // function prove_warp_symbolic(uint128 jump) public {
    //     test_warp_concrete(jump);
    // }


    function test_store_load_concrete(uint x) public {
        uint ten = uint(hevm.load(store, bytes32(0)));
        assertEq(ten, 10);

        hevm.store(store, bytes32(0), bytes32(x));
        uint val = uint(hevm.load(store, bytes32(0)));
        assertEq(val, x);
    }

    // function prove_store_load_symbolic(uint x) public {
    //     test_store_load_concrete(x);
    // }

    function test_sign_addr_digest(uint sk, bytes32 digest) public {
        if (sk == 0) return; // invalid key

        (uint8 v, bytes32 r, bytes32 s) = hevm.sign(sk, digest);
        address expected = hevm.addr(sk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected);
    }

    function test_sign_addr_message(uint sk, bytes memory message) public {
        test_sign_addr_digest(sk, keccak256(message));
    }

    function testFail_sign_addr(uint sk, bytes32 digest) public {
        uint badKey = sk + 1;

        (uint8 v, bytes32 r, bytes32 s) = hevm.sign(badKey, digest);
        address expected = hevm.addr(sk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected);
    }

    function testFail_addr_zero_sk() public {
        hevm.addr(0);
    }

    function test_addr() public {
        uint sk = 77814517325470205911140941194401928579557062014761831930645393041380819009408;
        address expected = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

        assertEq(hevm.addr(sk), expected);
    }

    function testFFI() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "echo";
        inputs[1] = "-n";
        inputs[2] = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000046163616200000000000000000000000000000000000000000000000000000000";

        bytes memory res = hevm.ffi(inputs);
        (string memory output) = abi.decode(res, (string));
        assertEq(output, "acab");
    }
}
