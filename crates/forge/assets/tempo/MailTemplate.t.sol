// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {ITIP20RolesAuth} from "tempo-std/interfaces/ITIP20RolesAuth.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";
import {StdTokens} from "tempo-std/StdTokens.sol";
import {Mail} from "../src/Mail.sol";

/// @notice Tests for direct mail sending (no signature verification).
contract MailTest is Test {
    ITIP20 public token;
    Mail public mail;

    address public constant ALICE = address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
    address public constant BOB = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);

    function setUp() public virtual {
        address feeToken = vm.envOr("TEMPO_FEE_TOKEN", StdTokens.PATH_USD_ADDRESS);
        StdPrecompiles.TIP_FEE_MANAGER.setUserToken(feeToken);

        token = ITIP20(
            StdPrecompiles.TIP20_FACTORY
                .createToken("testUSD", "tUSD", "USD", StdTokens.PATH_USD, address(this), bytes32(0))
        );

        ITIP20RolesAuth(address(token)).grantRole(token.ISSUER_ROLE(), address(this));

        mail = new Mail(token);
    }

    function test_SendMail() public {
        token.mint(ALICE, 100_000 * 10 ** token.decimals());

        Mail.Attachment memory attachment =
            Mail.Attachment({amount: 100 * 10 ** token.decimals(), memo: "Invoice #1234"});

        vm.startPrank(ALICE);
        token.approve(address(mail), attachment.amount);
        mail.sendMail(BOB, "Hello Bob, here is your invoice.", attachment);
        vm.stopPrank();

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

/// @notice Tests for relayed mail using the TIP-1020 SignatureVerifier precompile (requires T3).
/// forge-config: default.hardfork = "tempo:T3"
contract MailRelayTest is MailTest {
    // secp256k1 keys (used by vm.sign / vm.addr)
    uint256 internal constant ALICE_PK = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 internal constant BOB_PK = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;

    // P256 key (used by vm.signP256 / vm.publicKeyP256)
    uint256 internal constant CAROL_P256_PK = 0x1;
    address internal CAROL;
    bytes32 internal carolPubX;
    bytes32 internal carolPubY;

    uint256 internal constant P256_ORDER = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551;
    uint256 internal constant P256N_HALF = 0x7FFFFFFF800000007FFFFFFFFFFFFFFFDE737D56D38BCF4279DCE5617E3192A8;

    function setUp() public override {
        super.setUp();

        // Derive P256 public key and Tempo address for Carol
        (uint256 x, uint256 y) = vm.publicKeyP256(CAROL_P256_PK);
        carolPubX = bytes32(x);
        carolPubY = bytes32(y);
        CAROL = address(uint160(uint256(keccak256(abi.encodePacked(x, y)))));
    }

    /// @notice Relayed send with a secp256k1 signature — Alice signs, Bob delivers.
    function test_SendMailWithSecp256k1Signature() public {
        token.mint(ALICE, 100_000 * 10 ** token.decimals());

        Mail.Attachment memory attachment =
            Mail.Attachment({amount: 100 * 10 ** token.decimals(), memo: "Invoice #1234"});

        vm.prank(ALICE);
        token.approve(address(mail), attachment.amount);

        string memory message = "Hello Bob, here is your invoice.";
        bytes32 digest = mail.getDigest(ALICE, BOB, message, attachment);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ALICE_PK, digest);

        vm.prank(BOB);
        mail.sendMail(ALICE, BOB, message, attachment, abi.encodePacked(r, s, v));

        assertEq(token.balanceOf(BOB), attachment.amount);
        assertEq(mail.nonces(ALICE), 1);
    }

    /// @notice Relayed send with a P256 signature — Carol signs, Bob delivers.
    function test_SendMailWithP256Signature() public {
        token.mint(CAROL, 100_000 * 10 ** token.decimals());

        Mail.Attachment memory attachment =
            Mail.Attachment({amount: 100 * 10 ** token.decimals(), memo: "Invoice #1234"});

        vm.prank(CAROL);
        token.approve(address(mail), attachment.amount);

        string memory message = "Hello Bob, signed with P256.";
        bytes32 digest = mail.getDigest(CAROL, BOB, message, attachment);
        (bytes32 r, bytes32 s) = vm.signP256(CAROL_P256_PK, digest);
        s = _normalizeP256S(s);

        bytes memory sig = abi.encodePacked(uint8(0x01), r, s, carolPubX, carolPubY, uint8(0));

        vm.prank(BOB);
        mail.sendMail(CAROL, BOB, message, attachment, sig);

        assertEq(token.balanceOf(BOB), attachment.amount);
        assertEq(mail.nonces(CAROL), 1);
    }

    /// @notice Replaying the same signature fails (nonce incremented).
    function test_ReplayReverts() public {
        token.mint(ALICE, 100_000 * 10 ** token.decimals());

        Mail.Attachment memory attachment = Mail.Attachment({amount: 50 * 10 ** token.decimals(), memo: "tip"});

        vm.prank(ALICE);
        token.approve(address(mail), attachment.amount * 2);

        string memory message = "tip";
        bytes32 digest = mail.getDigest(ALICE, BOB, message, attachment);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ALICE_PK, digest);
        bytes memory sig = abi.encodePacked(r, s, v);

        mail.sendMail(ALICE, BOB, message, attachment, sig);

        vm.expectRevert();
        mail.sendMail(ALICE, BOB, message, attachment, sig);
    }

    /// @notice Submitting Bob's signature as Alice's fails.
    function test_WrongSignerReverts() public {
        Mail.Attachment memory attachment = Mail.Attachment({amount: 100, memo: "fake"});

        string memory message = "spoofed";
        bytes32 digest = mail.getDigest(ALICE, BOB, message, attachment);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(BOB_PK, digest);

        vm.expectRevert("invalid signature");
        mail.sendMail(ALICE, BOB, message, attachment, abi.encodePacked(r, s, v));
    }

    /// @dev Normalize P256 s to low-s form (required by the precompile).
    function _normalizeP256S(bytes32 s) internal pure returns (bytes32) {
        uint256 sVal = uint256(s);
        if (sVal > P256N_HALF) return bytes32(P256_ORDER - sVal);
        return s;
    }
}
