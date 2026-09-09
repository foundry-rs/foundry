# `TODO`/`FIXME` comments

**Severity**: `Info`
**ID**: `todo-comment`

Flags `TODO` and `FIXME` markers left in comments, which signal unfinished work or known
bugs that have not been resolved before the code reached production.

## What it does

Reports `TODO` and `FIXME` markers in line, block, and NatSpec comments, regardless of case.
This includes common forms such as `TODO:`, `FIXME(...)`, and a bare marker at the start
of a comment line. Ordinary filenames such as `todo.md` are not markers.

## Why restrict this?

`TODO` and `FIXME` comments are development notes. Shipping them into production contracts
signals incomplete work.

Development branches and tracked follow-up work may legitimately contain these markers. Keep
useful context and suppress the lint when the outstanding work is understood and acceptable.

## Example

```solidity
contract Vault {
    // TODO: implement access control
    function withdraw() public {}

    // FIXME: this check is wrong
    function deposit(uint256 amount) public {
        require(amount > 0);
    }
}
```

Use instead:

```solidity
contract Vault {
    function withdraw() public onlyOwner {}

    function deposit(uint256 amount) public {
        require(amount > 0, "zero amount");
    }
}
```
