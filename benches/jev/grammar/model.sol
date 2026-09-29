// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

interface GrammarVm {
    function prank(address sender) external;
}

/// A local accounting model with no external assets or network dependencies.
contract LocalLedger {
    error InsufficientBalance();

    mapping(address => uint256) public balanceOf;
    mapping(address => uint256) public depositedBalanceOf;

    constructor() {
        for (uint160 actor = 1; actor <= 3; actor++) {
            balanceOf[address(actor)] = 1000;
        }
    }

    function deposit(uint256 amount) external {
        if (amount > balanceOf[msg.sender]) revert InsufficientBalance();
        balanceOf[msg.sender] -= amount;
        depositedBalanceOf[msg.sender] += amount;
    }

    function withdraw(uint256 amount) external {
        if (amount > depositedBalanceOf[msg.sender]) revert InsufficientBalance();
        depositedBalanceOf[msg.sender] -= amount;
        balanceOf[msg.sender] += amount;
    }
}

/// Fixed transition oracles shared by generated and ordinary fuzz actions.
contract GrammarHandlerBase {
    GrammarVm internal constant vm = GrammarVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    LocalLedger public immutable ledger;
    uint256 public successes;
    uint256 public expectedReverts;
    uint256 public nonzeroSuccesses;

    constructor(LocalLedger ledger_) {
        ledger = ledger_;
    }

    function depositRaw(uint256 actor, uint256 amount) external {
        _action(false, address(uint160(1 + actor % 3)), amount);
    }

    function withdrawRaw(uint256 actor, uint256 amount) external {
        _action(true, address(uint160(1 + actor % 3)), amount);
    }

    function _action(bool withdrawal, address sender, uint256 amount) internal {
        uint256 freeBefore = ledger.balanceOf(sender);
        uint256 depositedBefore = ledger.depositedBalanceOf(sender);
        uint256 available = withdrawal ? depositedBefore : freeBefore;
        bytes memory data =
            withdrawal ? abi.encodeCall(LocalLedger.withdraw, (amount)) : abi.encodeCall(LocalLedger.deposit, (amount));
        // Compute all view expressions before prank: a view call must not consume it.
        vm.prank(sender);
        (bool ok, bytes memory result) = address(ledger).call(data);
        if (amount > available) {
            require(!ok, "expected insufficient balance");
            require(
                keccak256(result) == keccak256(abi.encodeWithSelector(LocalLedger.InsufficientBalance.selector)),
                "unexpected revert"
            );
            require(ledger.balanceOf(sender) == freeBefore, "revert changed free balance");
            require(ledger.depositedBalanceOf(sender) == depositedBefore, "revert changed deposit balance");
            expectedReverts++;
        } else {
            require(ok, "valid action reverted");
            uint256 freeAfter = withdrawal ? freeBefore + amount : freeBefore - amount;
            uint256 depositedAfter = withdrawal ? depositedBefore - amount : depositedBefore + amount;
            require(ledger.balanceOf(sender) == freeAfter, "incorrect free balance");
            require(ledger.depositedBalanceOf(sender) == depositedAfter, "incorrect deposit balance");
            successes++;
            if (amount != 0) nonzeroSuccesses++;
        }
        // Check after every inner action, not only at the end of a sequence.
        for (uint160 actor = 1; actor <= 3; actor++) {
            address owner = address(actor);
            require(ledger.balanceOf(owner) + ledger.depositedBalanceOf(owner) == 1000, "conservation");
        }
    }
}
