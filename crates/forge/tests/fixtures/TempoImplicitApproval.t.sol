// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.20;

import {Test, Vm} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
    function isImplicitlyApproved(address spender) external view returns (bool);
    function assumeImplicitApproval(address spender) external view;
}

interface IAddressRegistry {
    function isImplicitlyApproved(address addr) external view returns (bool);
}

interface ITIP20 {
    function approve(address spender, uint256 amount) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);

    event Transfer(address indexed from, address indexed to, uint256 amount);
    event Approval(address indexed owner, address indexed spender, uint256 amount);
}

interface IStablecoinDEX {
    function place(address token, uint128 amount, bool isBid, int16 tick)
        external
        returns (uint128 orderId);
    function MIN_ORDER_AMOUNT() external pure returns (uint128);
}

contract TempoImplicitApprovalTest is Test {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    // Well-known Tempo precompile addresses (mirrors of `tempo-contracts` constants).
    address constant TIP_FEE_MANAGER = 0xfeEC000000000000000000000000000000000000;
    address constant STABLECOIN_DEX = 0xDEc0000000000000000000000000000000000000;
    address constant TIP20_CHANNEL_RESERVE = 0x4d50500000000000000000000000000000000000;
    address constant ADDRESS_REGISTRY = 0xfDC0000000000000000000000000000000000000;
    address constant PATH_USD = 0x20C0000000000000000000000000000000000000;
    // AlphaUSD has `quote_token = PATH_USD`, so a DEX bid on ALPHA escrows PATH_USD.
    address constant ALPHA_USD = 0x20C0000000000000000000000000000000000001;

    // Foundry's default test sender; genesis mints the fee tokens to this address.
    address constant TEST_SENDER = 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38;

    /// On non-T5 specs the implicit list is empty.
    function test_cheatcode_isImplicitlyApproved_reports_pre_t5() public {
        evm.setEvmVersion("T4");
        assertEq(evm.getEvmVersion(), "t4");
        assertFalse(evm.isImplicitlyApproved(TIP_FEE_MANAGER));
        assertFalse(evm.isImplicitlyApproved(STABLECOIN_DEX));
        assertFalse(evm.isImplicitlyApproved(TIP20_CHANNEL_RESERVE));
        assertFalse(evm.isImplicitlyApproved(address(0xBEEF)));
    }

    /// On T5 the three listed precompiles are implicitly approved; nothing else is.
    function test_isImplicitlyApproved_t5() public view {
        assertTrue(evm.isImplicitlyApproved(TIP_FEE_MANAGER));
        assertTrue(evm.isImplicitlyApproved(STABLECOIN_DEX));
        assertTrue(evm.isImplicitlyApproved(TIP20_CHANNEL_RESERVE));
        assertFalse(evm.isImplicitlyApproved(address(0xBEEF)));
        assertFalse(evm.isImplicitlyApproved(TEST_SENDER));
    }

    /// The cheatcode and the on-chain AddressRegistry selector agree.
    function test_cheatcode_matches_registry_selector_t5() public view {
        address[4] memory probes = [
            TIP_FEE_MANAGER,
            STABLECOIN_DEX,
            TIP20_CHANNEL_RESERVE,
            address(0xBEEF)
        ];
        for (uint256 i = 0; i < probes.length; i++) {
            bool fromCheat = evm.isImplicitlyApproved(probes[i]);
            bool fromRegistry = IAddressRegistry(ADDRESS_REGISTRY).isImplicitlyApproved(probes[i]);
            assertEq(fromCheat, fromRegistry, "cheatcode and registry disagree");
        }
    }

    /// Standard `approve` + `allowance` + `transferFrom` and their events are unchanged.
    function test_standard_approve_flow_unchanged_t5() public {
        ITIP20 token = ITIP20(PATH_USD);

        address owner = TEST_SENDER;
        address spender = address(0xCAFE);
        address recipient = address(0xBEEF);

        vm.recordLogs();

        vm.prank(owner);
        assertTrue(token.approve(spender, 1_000));
        assertEq(token.allowance(owner, spender), 1_000);

        uint256 ownerBefore = token.balanceOf(owner);
        uint256 recipientBefore = token.balanceOf(recipient);

        vm.prank(spender);
        assertTrue(token.transferFrom(owner, recipient, 250));

        assertEq(token.balanceOf(owner), ownerBefore - 250);
        assertEq(token.balanceOf(recipient), recipientBefore + 250);
        assertEq(token.allowance(owner, spender), 750);

        Vm.Log[] memory entries = vm.getRecordedLogs();
        bytes32 transferSig = keccak256("Transfer(address,address,uint256)");
        bytes32 approvalSig = keccak256("Approval(address,address,uint256)");
        bool sawTransfer;
        bool sawApproval;
        for (uint256 i = 0; i < entries.length; i++) {
            if (entries[i].emitter != PATH_USD) continue;
            if (entries[i].topics.length == 0) continue;
            if (entries[i].topics[0] == transferSig) sawTransfer = true;
            if (entries[i].topics[0] == approvalSig) sawApproval = true;
        }
        assertTrue(sawTransfer, "standard Transfer event missing");
        assertTrue(sawApproval, "standard Approval event missing");
    }

    /// A bid on the DEX pulls the quote token from the sender with no prior approve, leaves
    /// allowance at zero, and still emits a standard `Transfer` event.
    function test_implicit_spender_pulls_without_approve_t5() public {
        IStablecoinDEX dex = IStablecoinDEX(STABLECOIN_DEX);
        ITIP20 quote = ITIP20(PATH_USD);

        address payer = TEST_SENDER;
        uint128 amount = dex.MIN_ORDER_AMOUNT();

        assertEq(quote.allowance(payer, STABLECOIN_DEX), 0, "precondition: no prior approval");

        uint256 payerBefore = quote.balanceOf(payer);
        uint256 dexBefore = quote.balanceOf(STABLECOIN_DEX);

        // Fail if any `approve(STABLECOIN_DEX, *)` is made on PATH_USD during the flow.
        vm.expectCall(
            PATH_USD,
            abi.encodeWithSelector(ITIP20.approve.selector, STABLECOIN_DEX),
            0
        );

        vm.recordLogs();

        // is_bid=true at tick=0 escrows `amount` units of the quote token (PATH_USD).
        vm.prank(payer);
        uint128 orderId = dex.place(ALPHA_USD, amount, true, 0);
        assertGt(uint256(orderId), 0, "order id must be non-zero");

        assertEq(quote.balanceOf(payer), payerBefore - amount, "payer balance not debited");
        assertEq(quote.balanceOf(STABLECOIN_DEX), dexBefore + amount, "DEX balance not credited");
        assertEq(quote.allowance(payer, STABLECOIN_DEX), 0, "allowance must remain zero");

        Vm.Log[] memory entries = vm.getRecordedLogs();
        bytes32 transferSig = keccak256("Transfer(address,address,uint256)");
        bool sawTransfer;
        for (uint256 i = 0; i < entries.length; i++) {
            if (entries[i].emitter != PATH_USD) continue;
            if (entries[i].topics.length == 0) continue;
            if (entries[i].topics[0] == transferSig) {
                sawTransfer = true;
                break;
            }
        }
        assertTrue(sawTransfer, "Transfer event missing");
    }

    /// Control: an unlisted spender still needs a prior approve.
    function test_non_implicit_spender_requires_approve_t5() public {
        ITIP20 token = ITIP20(PATH_USD);
        address payer = TEST_SENDER;
        address spender = address(0xBEEF);

        assertFalse(evm.isImplicitlyApproved(spender));
        assertEq(token.allowance(payer, spender), 0);

        vm.prank(spender);
        vm.expectRevert();
        token.transferFrom(payer, spender, 1);
    }

    /// `assumeImplicitApproval` is a no-op for a listed spender.
    function test_assumeImplicitApproval_positive_t5() public view {
        evm.assumeImplicitApproval(TIP_FEE_MANAGER);
    }
}
