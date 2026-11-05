// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Single-deal escrow between a buyer and a seller with an arbiter for disputes.
contract Escrow {
    enum State {
        Open,
        Funded,
        Shipped,
        Disputed,
        Closed
    }

    uint256 public constant CLAIM_DELAY = 7 days;

    address public immutable buyer;
    address public immutable seller;
    address public immutable arbiter;

    State public state;
    uint256 public amount;
    uint256 public shippedAt;

    constructor(address buyer_, address seller_, address arbiter_) {
        buyer = buyer_;
        seller = seller_;
        arbiter = arbiter_;
    }

    function fund() external payable {
        require(msg.sender == buyer && state == State.Open && msg.value > 0, "cannot fund");
        amount = msg.value;
        state = State.Funded;
    }

    function markShipped() external {
        require(msg.sender == seller && state == State.Funded, "cannot ship");
        shippedAt = block.timestamp;
        state = State.Shipped;
    }

    function confirmReceipt() external {
        require(msg.sender == buyer && state == State.Shipped, "cannot confirm");
        _close(seller);
    }

    function dispute() external {
        require(msg.sender == buyer && (state == State.Funded || state == State.Shipped), "cannot dispute");
        state = State.Disputed;
    }

    function resolve(bool refundBuyer) external {
        require(msg.sender == arbiter && state == State.Disputed, "cannot resolve");
        _close(refundBuyer ? buyer : seller);
    }

    function claimAfterTimeout() external {
        // BUG: a disputed deal must only be settled by the arbiter, but the
        // timeout path also accepts `Disputed`.
        require(msg.sender == seller && (state == State.Shipped || state == State.Disputed), "cannot claim");
        require(block.timestamp >= shippedAt + CLAIM_DELAY, "too early");
        _close(seller);
    }

    function reopen() external {
        require(state == State.Closed, "not closed");
        state = State.Open;
        amount = 0;
        shippedAt = 0;
    }

    function _close(address to) internal {
        state = State.Closed;
        (bool ok,) = to.call{value: amount}("");
        require(ok, "payout failed");
    }
}
