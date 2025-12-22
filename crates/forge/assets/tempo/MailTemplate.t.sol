// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {ITIP20RolesAuth} from "tempo-std/interfaces/ITIP20RolesAuth.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";
import {StdTokens} from "tempo-std/StdTokens.sol";
import {Mail} from "../src/Mail.sol";

contract MailTest is Test {
    ITIP20 public token;
    Mail public mail;

    address public constant ALICE = address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
    address public constant BOB = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);

    function setUp() public {
        StdPrecompiles.TIP_FEE_MANAGER.setUserToken(StdTokens.ALPHA_USD_ADDRESS);

        token = ITIP20(
            StdPrecompiles.TIP20_FACTORY.createToken("testUSD", "tUSD", "USD", StdTokens.PATH_USD, address(this))
        );

        ITIP20RolesAuth(address(token)).grantRole(token.ISSUER_ROLE(), address(this));

        mail = new Mail(token);
    }

    function test_SendMail() public {
        token.mint(ALICE, 100_000 * 10 ** token.decimals());

        Mail.Attachment memory attachment =
            Mail.Attachment({amount: 100 * 10 ** token.decimals(), memo: "Invoice #1234"});

        vm.prank(ALICE);
        token.approve(address(mail), attachment.amount);

        vm.prank(ALICE);
        mail.sendMail(BOB, "Hello Alice, this is a unit test mail.", attachment);

        assertEq(token.balanceOf(BOB), attachment.amount);
        assertEq(token.balanceOf(ALICE), 100_000 * 10 ** token.decimals() - attachment.amount);
    }

    function testFuzz_SendMail(uint128 mintAmount, uint128 sendAmount, string memory message, bytes32 memo) public {
        mintAmount = uint128(bound(mintAmount, 0, type(uint128).max));
        sendAmount = uint128(bound(sendAmount, 0, mintAmount));

        token.mint(ALICE, mintAmount);

        Mail.Attachment memory attachment = Mail.Attachment({amount: sendAmount, memo: memo});

        vm.startPrank(ALICE);
        token.approve(address(mail), sendAmount);
        mail.sendMail(BOB, message, attachment);
        vm.stopPrank();

        assertEq(token.balanceOf(BOB), sendAmount);
        assertEq(token.balanceOf(ALICE), mintAmount - sendAmount);
    }
}
