// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {ITIP20} from "tempo-std/interfaces/ITIP20.sol";
import {StdPrecompiles} from "tempo-std/StdPrecompiles.sol";

/// @title Mail
/// @notice Send mail with TIP-20 token attachments on Tempo.
///
/// Supports two modes:
///   1. Direct — call `sendMail()` yourself (uses `msg.sender`).
///   2. Relayed — sign a mail off-chain and let anyone deliver it on-chain.
///
/// Relayed mode uses the [TIP-1020] `SignatureVerifier` precompile to verify the
/// sender's Tempo signature. Unlike Ethereum's `ecrecover`, this precompile:
///   - Supports secp256k1, P256, and WebAuthn signature types
///   - Reverts on invalid signatures instead of returning `address(0)`
///   - Maintains forward compatibility with future Tempo account types
///
/// [TIP-1020]: <https://docs.tempo.xyz/protocol/tips/tip-1020>
contract Mail {
    /// @notice Emitted when a mail is sent, either directly or via a relayer.
    event MailSent(address indexed from, address indexed to, string message, Attachment attachment);

    /// @notice A TIP-20 token transfer bundled with a mail.
    struct Attachment {
        uint256 amount;
        bytes32 memo;
    }

    /// @notice The TIP-20 token used for mail attachments.
    ITIP20 public token;

    /// @notice Per-sender nonce to prevent signature replay on relayed mails (requires T3).
    mapping(address => uint256) public nonces;

    constructor(ITIP20 token_) {
        token = token_;
    }

    /// @notice Send mail directly (sender = msg.sender).
    function sendMail(address to, string memory message, Attachment memory attachment) external {
        token.transferFromWithMemo(msg.sender, to, attachment.amount, attachment.memo);
        emit MailSent(msg.sender, to, message, attachment);
    }

    /// @notice Send mail on behalf of `from` using their off-chain Tempo signature (requires T3).
    /// @dev The sender must have pre-approved this contract to spend their tokens.
    function sendMail(
        address from,
        address to,
        string memory message,
        Attachment memory attachment,
        bytes calldata signature
    ) external {
        bytes32 hash = getDigest(from, to, message, attachment);

        // `verify()` returns `false` on signer mismatch, reverts on malformed signatures.
        require(StdPrecompiles.SIGNATURE_VERIFIER.verify(from, hash, signature), "invalid signature");

        // `recover()` returns the signer address directly, reverts on malformed signatures.
        require(StdPrecompiles.SIGNATURE_VERIFIER.recover(hash, signature) == from, "invalid signature");

        nonces[from]++;
        token.transferFromWithMemo(from, to, attachment.amount, attachment.memo);
        emit MailSent(from, to, message, attachment);
    }

    /// @notice Compute the digest a sender must sign to authorize a relayed mail.
    function getDigest(address from, address to, string memory message, Attachment memory attachment)
        public
        view
        returns (bytes32)
    {
        return keccak256(abi.encode(address(this), block.chainid, from, to, message, attachment, nonces[from]));
    }
}
