// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ContextFreeUtilitiesTest is Test {
    function testNamehash() public {
        assertEq(vm.ensNamehash(""), bytes32(0));
        bytes32 eth = 0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae;
        assertEq(vm.ensNamehash("foo.eth"), 0xde9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f);
        // Namehash hashes labels verbatim, including malformed empty labels; it is not a validator.
        assertEq(vm.ensNamehash(".eth"), keccak256(abi.encodePacked(eth, keccak256(""))));
    }

    function testSort() public {
        uint256[] memory input = new uint256[](5);
        input[0] = type(uint256).max;
        input[1] = 9;
        input[2] = 0;
        input[3] = 9;
        input[4] = 2;
        uint256[] memory result = vm.sort(input);
        assertEq(result.length, 5);
        assertEq(result[0], 0);
        assertEq(result[1], 2);
        assertEq(result[2], 9);
        assertEq(result[3], 9);
        assertEq(result[4], type(uint256).max);
        assertEq(vm.sort(new uint256[](0)).length, 0);
    }

    function testRlp() public {
        bytes[] memory data = new bytes[](3);
        data[0] = hex"";
        data[1] = hex"7f";
        data[2] = hex"80ff";
        assertEq(vm.toRlp(data), hex"c5807f8280ff");
        bytes[] memory decoded = vm.fromRlp(hex"c5807f8280ff");
        assertEq(decoded.length, 3);
        assertEq(decoded[0], hex"");
        assertEq(decoded[1], hex"7f");
        assertEq(decoded[2], hex"80ff");
        assertEq(vm.toRlp(new bytes[](0)), hex"c0");
        assertEq(vm.fromRlp(hex"c0").length, 0);
        vm._expectCheatcodeRevert();
        vm.fromRlp(hex"c28201");
        vm._expectCheatcodeRevert();
        vm.fromRlp(hex"80");
        vm._expectCheatcodeRevert();
        vm.fromRlp(hex"c1c0");
    }

    function testTypedData() public {
        // The canonical EIP-712 Ether Mail example, with distinct sender and recipient.
        string memory json =
            '{"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],"Person":[{"name":"name","type":"string"},{"name":"wallet","type":"address"}],"Mail":[{"name":"from","type":"Person"},{"name":"to","type":"Person"},{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"Ether Mail","version":"1","chainId":1,"verifyingContract":"0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"},"message":{"from":{"name":"Cow","wallet":"0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},"to":{"name":"Bob","wallet":"0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},"contents":"Hello, Bob!"}}';
        assertEq(vm.eip712HashTypedData(json), 0xbe609aee343fb3c4b28e1df9e632fca64fcfaede20f02e86244efddf30957bd2);
        vm._expectCheatcodeRevert();
        vm.eip712HashTypedData("not JSON");
    }

    function testHostTimeAndVersion() public {
        uint256 start = vm.unixTime();
        assertGt(start, 1700000000000);
        vm.sleep(25);
        uint256 elapsed = vm.unixTime() - start;
        assertGe(elapsed, 25);
        assertLt(elapsed, 10000);
        vm.sleep(0);
        string[] memory parts = vm.split(vm.getFoundryVersion(), "+");
        assertEq(parts.length, 2);
        string[] memory version = vm.split(vm.split(parts[0], "-")[0], ".");
        assertEq(version.length, 3);
        vm.parseUint(version[0]);
        vm.parseUint(version[1]);
        vm.parseUint(version[2]);
        assertGt(bytes(parts[1]).length, 0);
    }
}
