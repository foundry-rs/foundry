contract C {
    function f() external {
        for (uint i = 0; i < targets.length; i++) {
            if (ITimelock(localTimelock).queuedTransactions(keccak256(abi.encode(targets[i], values[i], signatures[i], calldatas[i], eta)))) revert TransactionAlreadyQueued();
            ITimelock(localTimelock).queueTransaction(targets[i], values[i], signatures[i], calldatas[i], eta);
        }
    }
}
