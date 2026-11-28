// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.20;

interface Vm {
    function deal(address account, uint256 balance) external;
    function prank(address sender) external;
}

contract ArcEvmTest {
    Vm internal constant vm = Vm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));
    address internal constant NATIVE_FIAT_TOKEN = 0x3600000000000000000000000000000000000000;
    address internal constant NATIVE_COIN_CONTROL = 0x1800000000000000000000000000000000000001;
    address internal constant CALL_FROM = 0x1800000000000000000000000000000000000003;
    address internal constant RECIPIENT = address(0xBEEF);

    function setUp() public {
        vm.deal(address(this), 1 ether);
    }

    function test_precompile_accounts_hold_code() public view {
        require(NATIVE_COIN_CONTROL.code.length == 1, "precompile account has no code");
        require(CALL_FROM.code.length == 1, "callFrom account has no code");
    }

    function test_value_to_zero_address_reverts() public {
        (bool success, bytes memory output) = payable(address(0)).call{value: 1}("");
        require(!success, "value transfer to the zero address succeeded");
        require(keccak256(output) == keccak256(abi.encodeWithSignature("Error(string)", "Zero address not allowed")));
    }

    function test_value_to_blocklisted_address_reverts() public {
        // Only the USDC token may update the blocklist.
        vm.prank(NATIVE_FIAT_TOKEN);
        (bool success, bytes memory output) = NATIVE_COIN_CONTROL.call(abi.encodeWithSignature("blocklist(address)", RECIPIENT));
        require(success, "blocklist failed");

        (success, output) = payable(RECIPIENT).call{value: 1}("");
        require(!success, "value transfer to a blocklisted address succeeded");
        require(keccak256(output) == keccak256(abi.encodeWithSignature("Error(string)", "Blocked address")));
        require(RECIPIENT.balance == 0, "blocklisted address was credited");

        (success,) = payable(RECIPIENT).call("");
        require(success, "call without value to a blocklisted address failed");
    }

    function test_call_from_rejects_unauthorized_callers() public {
        (bool success, bytes memory output) = CALL_FROM.call(abi.encodeWithSignature("callFrom(address,address,bytes)", address(this), RECIPIENT, ""));
        require(!success, "callFrom accepted an unauthorized caller");
        require(keccak256(output) == keccak256(abi.encodeWithSignature("Error(string)", "unauthorized caller")));
    }

    function test_call_from_is_absent_before_zero7() public {
        // Before Zero7 the account only runs its `0xef` code, which halts.
        (bool success, bytes memory output) = CALL_FROM.call(abi.encodeWithSignature("callFrom(address,address,bytes)", address(this), RECIPIENT, ""));
        require(!success && output.length == 0, "callFrom is installed");
    }
}
