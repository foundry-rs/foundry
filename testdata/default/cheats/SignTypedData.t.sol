// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SignTypedDataTest is Test {
    /// The `Mail` example of the EIP-712 specification.
    string constant MAIL =
        '{"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],"Person":[{"name":"name","type":"string"},{"name":"wallet","type":"address"}],"Mail":[{"name":"from","type":"Person"},{"name":"to","type":"Person"},{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"Ether Mail","version":"1","chainId":1,"verifyingContract":"0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"},"message":{"from":{"name":"Cow","wallet":"0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},"to":{"name":"Bob","wallet":"0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},"contents":"Hello, Bob!"}}';

    function testSignTypedData() public {
        // The EIP-712 specification signs its example with the private key `keccak256("cow")`:
        // <https://github.com/ethereum/EIPs/blob/master/assets/eip-712/Example.js>
        uint256 pk = uint256(keccak256("cow"));
        assertEq(vm.addr(pk), 0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826);

        (uint8 v, bytes32 r, bytes32 s) = vm.signTypedData(pk, MAIL);
        assertEq(v, 28);
        assertEq(r, 0x4355c47d63924e8a72e509b65029052eb6c299d53a04e167c5775fd466751c9d);
        assertEq(s, 0x07299936d304c153f6443dfa05f40ff007d72911b6f72307f996231605b91562);
    }

    function testSignTypedDataMatchesSignedDigest(uint248 pk) public {
        vm.assume(pk != 0);

        bytes32 digest = vm.eip712HashTypedData(MAIL);
        (uint8 v, bytes32 r, bytes32 s) = vm.signTypedData(pk, MAIL);
        assertEq(ecrecover(digest, v, r, s), vm.addr(pk), "typed data signer did not match");

        (uint8 expectedV, bytes32 expectedR, bytes32 expectedS) = vm.sign(pk, digest);
        assertEq(v, expectedV);
        assertEq(r, expectedR);
        assertEq(s, expectedS);
    }

    function testSignTypedDataRevertsOnInvalidTypedData() public {
        vm._expectCheatcodeRevert();
        vm.signTypedData(1, "{}");

        vm._expectCheatcodeRevert();
        vm.signTypedData(1, "not json");
    }

    function testSignTypedDataRevertsOnInvalidPrivateKey() public {
        vm._expectCheatcodeRevert("vm.signTypedData: private key cannot be 0");
        vm.signTypedData(0, MAIL);
    }

    function testSignTypedDataRevertsWithoutScriptWallets() public {
        vm._expectCheatcodeRevert("vm.signTypedData: no wallets available");
        vm.signTypedData(MAIL);

        vm._expectCheatcodeRevert("vm.signTypedData: no wallets available");
        vm.signTypedData(address(this), MAIL);
    }
}
