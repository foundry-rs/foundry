// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Tip20Like {
    uint256[5] private __gap;
    string public logoURI;

    event LogoURIUpdated(address indexed updater, string newLogoURI);

    function emitLogoURIUpdated(string memory newLogoURI) external {
        emit LogoURIUpdated(msg.sender, newLogoURI);
    }
}

contract TIP20LogoURITest is Test {
    uint256 internal constant LOGO_URI_SLOT = 5;

    function testSetTip20LogoURIWritesTip20Slot() public {
        Tip20Like token = new Tip20Like();
        string memory longUri = string.concat("https://example.com/", _repeat("a", 80));

        vm.setTip20LogoURI(address(token), longUri);
        assertEq(token.logoURI(), longUri, "long logo URI mismatch");

        vm.setTip20LogoURI(address(token), "ipfs://token");
        assertEq(token.logoURI(), "ipfs://token", "short logo URI mismatch");

        vm.setTip20LogoURI(address(token), "");
        assertEq(token.logoURI(), "", "empty logo URI mismatch");
    }

    function testSetTip20LogoURIStorageBoundaries() public {
        Tip20Like token = new Tip20Like();

        for (uint256 i; i < 6; i++) {
            uint256 length = _boundaryLength(i);
            string memory uri = _uriWithLength(length);

            vm.setTip20LogoURI(address(token), uri);

            assertEq(bytes(token.logoURI()).length, length, "logo URI length mismatch");
            assertEq(token.logoURI(), uri, "logo URI value mismatch");
        }
    }

    function testSetTip20LogoURIAcceptsExactByteLimit() public {
        Tip20Like token = new Tip20Like();
        string memory uri = _uriWithLength(256);

        vm.setTip20LogoURI(address(token), uri);

        assertEq(bytes(token.logoURI()).length, 256, "256-byte logo URI rejected");
        assertEq(token.logoURI(), uri, "256-byte logo URI mismatch");
    }

    function testSetTip20LogoURICleansStaleLongStringTail() public {
        Tip20Like token = new Tip20Like();

        vm.setTip20LogoURI(address(token), _uriWithLength(65));
        bytes32 chunk0 = vm.load(address(token), _logoDataSlot(0));
        bytes32 chunk1 = vm.load(address(token), _logoDataSlot(1));
        bytes32 chunk2 = vm.load(address(token), _logoDataSlot(2));
        assertTrue(chunk0 != bytes32(0), "long URI chunk 0 not written");
        assertTrue(chunk1 != bytes32(0), "long URI chunk 1 not written");
        assertTrue(chunk2 != bytes32(0), "long URI chunk 2 not written");

        vm.setTip20LogoURI(address(token), _uriWithLength(33));
        assertEq(vm.load(address(token), _logoDataSlot(2)), bytes32(0), "long-to-long tail not cleared");

        vm.setTip20LogoURI(address(token), _uriWithLength(31));
        assertEq(vm.load(address(token), _logoDataSlot(0)), bytes32(0), "long-to-short chunk 0 not cleared");
        assertEq(vm.load(address(token), _logoDataSlot(1)), bytes32(0), "long-to-short chunk 1 not cleared");
    }

    function testSetTip20LogoURIOverwritesMalformedPriorSlot() public {
        Tip20Like token = new Tip20Like();
        string memory uri = "https://example.com/logo.svg";

        vm.store(address(token), bytes32(LOGO_URI_SLOT), bytes32(uint256(515)));
        vm.setTip20LogoURI(address(token), uri);

        assertEq(token.logoURI(), uri, "malformed prior slot blocked overwrite");
    }

    function testSetTip20LogoURIWritesRawStorageEncoding() public {
        Tip20Like token = new Tip20Like();
        string memory shortUri = _uriWithLength(31);
        string memory longUri = _uriWithLength(32);

        vm.setTip20LogoURI(address(token), shortUri);
        assertEq(vm.load(address(token), bytes32(LOGO_URI_SLOT)), _shortStorageValue(shortUri), "short slot encoding");

        vm.setTip20LogoURI(address(token), longUri);
        assertEq(
            vm.load(address(token), bytes32(LOGO_URI_SLOT)),
            bytes32(uint256(bytes(longUri).length * 2 + 1)),
            "long slot length tag"
        );
        assertEq(vm.load(address(token), _logoDataSlot(0)), _stringChunk(longUri, 0), "long slot data chunk");
    }

    function testSetTip20LogoURIRejectsInvalidValues() public {
        Tip20Like token = new Tip20Like();

        vm._expectCheatcodeRevert("InvalidLogoURI");
        vm.setTip20LogoURI(address(token), "ftp://example.com/logo.png");

        vm._expectCheatcodeRevert("InvalidLogoURI");
        vm.setTip20LogoURI(address(token), "example.com/logo.png");

        vm._expectCheatcodeRevert("LogoURITooLong");
        vm.setTip20LogoURI(address(token), string.concat("https://", _repeat("a", 249)));
    }

    function testSetTip20LogoURIAcceptsMixedCaseScheme() public {
        Tip20Like token = new Tip20Like();

        vm.setTip20LogoURI(address(token), "HTTPS://example.com/logo.svg");

        assertEq(token.logoURI(), "HTTPS://example.com/logo.svg", "mixed-case scheme rejected");
    }

    function testSetLogoURIAliasWritesTip20Slot() public {
        Tip20Like token = new Tip20Like();
        string memory uri = "https://example.com/alias.svg";

        vm.setLogoURI(address(token), uri);

        assertEq(token.logoURI(), uri, "alias logo URI mismatch");
    }

    function testSetTip20LogoURIRejectsPrecompiles() public {
        vm._expectCheatcodeRevert("cannot use precompile 0x0000000000000000000000000000000000000001 as an argument");
        vm.setTip20LogoURI(address(1), "https://example.com/logo.svg");
    }

    function testExpectTip20LogoURIUpdated() public {
        Tip20Like token = new Tip20Like();
        string memory uri = "https://example.com/logo.svg";

        vm.expectTip20LogoURIUpdated(address(token), address(this), uri);
        token.emitLogoURIUpdated(uri);
    }

    function testExpectLogoURIUpdatedAlias() public {
        Tip20Like token = new Tip20Like();
        string memory uri = "https://example.com/alias.svg";

        vm.expectLogoURIUpdated(address(token), address(this), uri);
        token.emitLogoURIUpdated(uri);
    }

    function _repeat(string memory value, uint256 count) private pure returns (string memory out) {
        for (uint256 i; i < count; i++) {
            out = string.concat(out, value);
        }
    }

    function _boundaryLength(uint256 index) private pure returns (uint256) {
        uint256[6] memory lengths = [uint256(30), 31, 32, 33, 64, 65];
        return lengths[index];
    }

    function _uriWithLength(uint256 length) private pure returns (string memory) {
        string memory prefix = "https://";
        require(length >= bytes(prefix).length, "invalid length");
        return string.concat(prefix, _repeat("a", length - bytes(prefix).length));
    }

    function _logoDataSlot(uint256 offset) private pure returns (bytes32) {
        return bytes32(uint256(keccak256(abi.encode(LOGO_URI_SLOT))) + offset);
    }

    function _shortStorageValue(string memory value) private pure returns (bytes32 slotValue) {
        bytes memory raw = bytes(value);
        require(raw.length <= 31, "not short");
        assembly {
            slotValue := mload(add(raw, 32))
        }
        return bytes32((uint256(slotValue) & ~uint256(0xff)) | (raw.length * 2));
    }

    function _stringChunk(string memory value, uint256 offset) private pure returns (bytes32 chunk) {
        bytes memory raw = bytes(value);
        require(offset + 32 <= raw.length, "chunk out of bounds");
        assembly {
            chunk := mload(add(add(raw, 32), offset))
        }
    }
}
