//@compile-flags: --only-lint event-fields

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Top-level event with unindexed address.
event TopLevelTransfer(address from, address to, uint256 value); //~NOTE: event has unindexed fields that should be indexed: from (address), to (address)

// Top-level event already fully OK.
event TopLevelOk(address indexed from, address indexed to, uint256 value);

interface IEvents {
    // Interface event with unindexed address.
    event InterfaceTransfer(address from, uint256 value); //~NOTE: event has unindexed fields that should be indexed: from (address)
}

library LibEvents {
    // Library event with unindexed id.
    event LibCreated(uint256 id, uint256 amount); //~NOTE: event has unindexed fields that should be indexed: id (uint256)
}

contract EventFieldsTest {
    // --- triggering cases -------------------------------------------------

    event Transfer(address from, address to, uint256 value); //~NOTE: event has unindexed fields that should be indexed: from (address), to (address)

    event Mint(address to, uint256 tokenId); //~NOTE: event has unindexed fields that should be indexed: to (address), tokenId (uint256)

    event Order(bytes32 orderId, uint256 amount); //~NOTE: event has unindexed fields that should be indexed: orderId (bytes32)

    event CreatedAlias(uint id); //~NOTE: event has unindexed fields that should be indexed: id (uint256)

    event ScreamingId(uint256 ID); //~NOTE: event has unindexed fields that should be indexed: ID (uint256)

    event SnakeId(bytes32 token_id); //~NOTE: event has unindexed fields that should be indexed: token_id (bytes32)

    event PayableAddr(address payable receiver); //~NOTE: event has unindexed fields that should be indexed: receiver (address payable)

    // Anonymous events allow up to 4 indexed.
    event AnonFour(address a, address b, address c, address d) anonymous; //~NOTE: event has unindexed fields that should be indexed: a (address), b (address), c (address), d (address)

    // Unnamed param is reported using its positional index.
    event Unnamed(address, uint256); //~NOTE: event has unindexed fields that should be indexed: parameter #1 (address)

    // --- non-triggering cases --------------------------------------------

    // Already fully indexed.
    event TransferOk(address indexed from, address indexed to, uint256 value);

    // Partially indexed: the author has chosen what to index, so we stay silent.
    // Matches Slither's behavior; figtracer's regression case from #14751.
    event PartiallyIndexed(address indexed from, address to, uint256 value);

    // Same rule for non-anonymous partial cap: at least one indexed param ⇒ no warning.
    event PartialCap(address indexed a, address indexed b, address c, address d);

    // Same rule for anonymous events.
    event AnonPartial(address indexed a, address indexed b, address indexed c, address d, address e) anonymous;

    // Non-id uint/bytes are not flagged.
    event AmountOnly(uint256 amount);
    event HashOnly(bytes32 hash);

    // Conservative heuristic: do not flag names containing "id" as a substring.
    event NoFalsePositive(uint256 liquid, bytes32 valid);

    // Smaller integer / bytes widths are not flagged even when id-like.
    event SmallId(uint128 id, bytes16 orderId);

    // Arrays and other non-elementary kinds are out of scope.
    event Arrays(address[] users, uint256[] ids, bytes32[] orderIds);

    // No params.
    event NoParams();

    // Capacity already full (3 indexed in non-anonymous): nothing actionable.
    event FullCap(address indexed a, address indexed b, address indexed c, address d);

    // Anonymous capacity full (4 indexed): nothing actionable.
    event AnonFullCap(address indexed a, address indexed b, address indexed c, address indexed d, address e) anonymous;
}

// Custom types (UDVT and contract-typed) are out of scope: AST pass cannot resolve them.
type UserId is uint256;
contract Token {}
event WrappedUdvt(UserId userId);
event ContractTyped(Token tok);
