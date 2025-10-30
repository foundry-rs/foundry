// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Rlp is Test {
    function testToRlp() public {
        bytes[] memory data = new bytes[](2);
        data[0] = hex"01";
        data[1] = hex"02";

        bytes memory rlp = vm.toRlp(data);

        // Assert the expected RLP encoding for [0x01, 0x02]
        // 0xc2 = list with 2 bytes total length
        // 0x01 = first byte
        // 0x02 = second byte
        assertEq(rlp, hex"c20102");
    }

    function testFromRlp() public {
        // RLP encoded [0x01, 0x02]
        bytes memory rlp = hex"c20102";

        bytes[] memory decoded = vm.fromRlp(rlp);
        assertEq(decoded.length, 2);
        assertEq(decoded[0], hex"01");
        assertEq(decoded[1], hex"02");
    }

    function testRoundTrip() public {
        bytes[] memory original = new bytes[](3);
        original[0] = hex"deadbeef";
        original[1] = hex"cafebabe";
        original[2] = hex"01020304";

        bytes memory rlp = vm.toRlp(original);
        bytes[] memory decoded = vm.fromRlp(rlp);

        assertEq(decoded.length, original.length);
        for (uint256 i = 0; i < original.length; i++) {
            assertEq(decoded[i], original[i]);
        }
    }

    function testDecodeBlockHeader() public {
        // cast block 23270177  --raw
        bytes memory blockHeader =
            hex"f9027da01581f4448b16694d5a728161cd65f8c80b88f5352a6f5bd2d2315b970582958da01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794dadb0d80178819f2319190d340ce9a924f783711a010d2afa5dabcf2dbfe3aa82b758427938e07880bd6fef3c82c404d0dd7c3f0f3a0f81230c715a462c827898bf2e337982907a7af90e5be20f911785bda05dab93ca0740f11bc75cf25e40d78d892d2e03083eaa573e5b4c26913fcc1b833db854c94b9010085f734fb06ea8fe377abbcb2e27f9ac99751ba817dc327327db101fd76f964ed0b7ca161f148fc165b9e5b575dc7473f17f4b8ebbf4a7b02b3e1e642197f27b2af54680834449abaf833619ac7d18afb50b19d5f6944dca0dc952edfdd9837573783c339ee6a36353ce6e536eaaf29fcd569c426091d4e24568dc353347f98c74fb6f8c91d68d358467c437563f66566377fe6c3f9e8301dbeb5fc7e7adee7a85ef5f8fa905cedbaf26601e21ba91646cac4034601e51d889d49739ee6990943a6a41927660f68e1f50b9f9209ee29551a7dae478d88e0547eefc83334ea770bb6fbac620fc47479c2c59389622bf32f55e36a75e56a5fc47c38bf8ef211fc0e8084016313218402af50e883fc53b78468b5ea9b974275696c6465724e657420284e65746865726d696e6429a0580ca94e91c0e7aef26ffb0c86f6ae48ef40df6dd1629f203a1930e0ce0be9d188000000000000000084479c1e2aa00345740e1b79edb2fbb3a20220e1a497ea9bb82aaba7dc7a881f7f3cae8a8ea38080a06675ad2a40134499a753924a04b75898ae09efc6fba6b3d7a506203042cb7611a0e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

        bytes[] memory decoded = vm.fromRlp(blockHeader);

        // Verify key fields against known values from block 23270177
        assertEq(decoded.length, 21);
        assertEq(decoded[0], hex"1581f4448b16694d5a728161cd65f8c80b88f5352a6f5bd2d2315b970582958d", "parentHash");
        assertEq(decoded[1], hex"1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347", "uncleHash");
        assertEq(decoded[2], hex"dadb0d80178819f2319190d340ce9a924f783711", "coinbase");
        assertEq(decoded[3], hex"10d2afa5dabcf2dbfe3aa82b758427938e07880bd6fef3c82c404d0dd7c3f0f3", "stateRoot");
        assertEq(decoded[4], hex"f81230c715a462c827898bf2e337982907a7af90e5be20f911785bda05dab93c", "transactionsRoot");
        assertEq(decoded[5], hex"740f11bc75cf25e40d78d892d2e03083eaa573e5b4c26913fcc1b833db854c94", "receiptsRoot");

        {
            // Verify logsBloom (256 bytes)
            bytes memory logsBloom = decoded[6];
            assertEq(logsBloom.length, 256, "logsBloom length");
            _checkLogsBloom(logsBloom, 0, 0x85f734fb06ea8fe377abbcb2e27f9ac99751ba817dc327327db101fd76f964ed);
            _checkLogsBloom(logsBloom, 1, 0x0b7ca161f148fc165b9e5b575dc7473f17f4b8ebbf4a7b02b3e1e642197f27b2);
            _checkLogsBloom(logsBloom, 2, 0xaf54680834449abaf833619ac7d18afb50b19d5f6944dca0dc952edfdd983757);
            _checkLogsBloom(logsBloom, 3, 0x3783c339ee6a36353ce6e536eaaf29fcd569c426091d4e24568dc353347f98c7);
            _checkLogsBloom(logsBloom, 4, 0x4fb6f8c91d68d358467c437563f66566377fe6c3f9e8301dbeb5fc7e7adee7a8);
            _checkLogsBloom(logsBloom, 5, 0x5ef5f8fa905cedbaf26601e21ba91646cac4034601e51d889d49739ee6990943);
            _checkLogsBloom(logsBloom, 6, 0xa6a41927660f68e1f50b9f9209ee29551a7dae478d88e0547eefc83334ea770b);
            _checkLogsBloom(logsBloom, 7, 0xb6fbac620fc47479c2c59389622bf32f55e36a75e56a5fc47c38bf8ef211fc0e);
        }

        assertEq(decoded[7], hex"", "difficulty");
        assertEq(decoded[8], hex"01631321", "number");
        assertEq(decoded[9], hex"02af50e8", "gasLimit");
        assertEq(decoded[10], hex"fc53b7", "gasUsed");
        assertEq(decoded[11], hex"68b5ea9b", "timestamp");
        assertEq(decoded[12], hex"4275696c6465724e657420284e65746865726d696e6429", "extraData");
        assertEq(decoded[13], hex"580ca94e91c0e7aef26ffb0c86f6ae48ef40df6dd1629f203a1930e0ce0be9d1", "mixHash");
        assertEq(decoded[14], hex"0000000000000000", "nonce");
        assertEq(decoded[15], hex"479c1e2a", "baseFee");
        assertEq(decoded[16], hex"0345740e1b79edb2fbb3a20220e1a497ea9bb82aaba7dc7a881f7f3cae8a8ea3", "withdrawalsHash");
        assertEq(decoded[17], hex"", "blobGasUsed");
        assertEq(decoded[18], hex"", "excessBlobGas");
        assertEq(decoded[19], hex"6675ad2a40134499a753924a04b75898ae09efc6fba6b3d7a506203042cb7611", "parentBeaconRoot");
        assertEq(decoded[20], hex"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "requestsHash");
    }

    function _checkLogsBloom(bytes memory data, uint256 n, uint256 expected) internal {
        uint256 offset = (n + 1) * 32;
        bytes32 result;
        assembly {
            result := mload(add(data, offset))
        }

        assertEq(result, bytes32(expected), string.concat("logsBloom[", vm.toString(n), "]"));
    }
}
