// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.24 <0.9.0;

// The CLI test appends the decoder emitted by the preprocessor and replaces its name here.
contract AddressDecoderTest {
    function decode(uint256 raw) external pure returns (address) {
        return DECODE_DEPLOYMENT_ADDRESS(raw);
    }

    function test_valid_deployment_addresses() public view {
        require(this.decode(0) == address(0), "zero address");
        require(this.decode(0x1234) == address(0x1234), "deployment address");
        require(this.decode(type(uint160).max) == address(type(uint160).max), "maximum address");
    }

    function test_noncanonical_deployment_addresses() public {
        (bool success, bytes memory result) =
            address(this).call(abi.encodeCall(this.decode, (uint256(type(uint160).max) + 1)));
        require(!success && result.length == 0, "truncated high address bits");
        (success, result) = address(this).call(abi.encodeCall(this.decode, (type(uint256).max)));
        require(!success && result.length == 0, "truncated maximum word");
    }

    function decodeReturnedAddress(address target) external returns (address) {
        return DECODE_DEPLOYMENT_ADDRESS(DeploymentWordReturn(target).read());
    }

    function nativeReturnedAddress(address target) external returns (address) {
        return DeploymentAddressReturn(target).read();
    }

    function test_short_deployment_return_data() public {
        DeploymentReturnData target = new DeploymentReturnData();
        uint256[3] memory lengths = [uint256(0), 1, 31];
        for (uint256 i; i < lengths.length; ++i) {
            target.setResponse(0x1234, lengths[i]);
            checkRejectedReturnData(address(target));
        }
    }

    function test_valid_deployment_return_data() public {
        DeploymentReturnData target = new DeploymentReturnData();
        uint256[3] memory words = [uint256(0), 0x1234, type(uint160).max];
        uint256[3] memory lengths = [uint256(32), 33, 64];
        for (uint256 i; i < words.length; ++i) {
            for (uint256 j; j < lengths.length; ++j) {
                target.setResponse(words[i], lengths[j]);
                address expected = address(uint160(words[i]));
                require(this.decodeReturnedAddress(address(target)) == expected, "decoded return data");
                require(this.nativeReturnedAddress(address(target)) == expected, "native return data");
            }
        }
    }

    function test_noncanonical_deployment_return_data() public {
        DeploymentReturnData target = new DeploymentReturnData();
        uint256[2] memory words = [uint256(type(uint160).max) + 1, type(uint256).max];
        uint256[3] memory lengths = [uint256(32), 33, 64];
        for (uint256 i; i < words.length; ++i) {
            for (uint256 j; j < lengths.length; ++j) {
                target.setResponse(words[i], lengths[j]);
                checkRejectedReturnData(address(target));
            }
        }
    }

    function checkRejectedReturnData(address target) internal {
        (bool success, bytes memory result) = address(this).call(abi.encodeCall(this.decodeReturnedAddress, (target)));
        require(!success && result.length == 0, "decoded invalid return data");
        (success, result) = address(this).call(abi.encodeCall(this.nativeReturnedAddress, (target)));
        require(!success && result.length == 0, "native invalid return data");
    }
}

interface DeploymentWordReturn {
    function read() external returns (uint256);
}

interface DeploymentAddressReturn {
    function read() external returns (address);
}

contract DeploymentReturnData {
    uint256 internal word;
    uint256 internal length;

    function setResponse(uint256 word_, uint256 length_) external {
        require(length_ <= 64, "test return data length");
        word = word_;
        length = length_;
    }

    fallback() external {
        uint256 word_ = word;
        uint256 length_ = length;
        assembly ("memory-safe") {
            mstore(0, word_)
            mstore(32, not(0))
            return(0, length_)
        }
    }
}
