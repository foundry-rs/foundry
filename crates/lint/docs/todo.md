# TODO/FIXME comments

**Severity**: `Info`
**ID**: `todo`

Flags `TODO` and `FIXME` markers left in comments, which signal unfinished work or known
bugs that have not been resolved before the code reached production.

## What it does

Scans every comment in the source file (single-line `//`, block `/* */`, and NatSpec `///`)
and reports any comment containing a `TODO` or `FIXME` word, matched case-insensitively
(e.g. `todo`, `ToDo`, `FixMe` all match). Matching is done on whole words, so a marker
must appear as its own word rather than as part of another word (e.g. `autodoc` does not
match). Markers inside string literals are not flagged.

A comment is reported once, even if it contains multiple markers; the diagnostic lists
each marker using the exact casing written in the source.

## Why is this bad?

`TODO` and `FIXME` comments are development notes. Shipping them into production contracts signals incomplete work.

## Example

### Bad

```solidity
contract Vault {
    // TODO: implement access control
    function withdraw() public {}

    // FIXME this check is wrong
    function deposit(uint256 amount) public {
        require(amount > 0);
    }
}
```

### Good

```solidity
contract Vault {
    function withdraw() public onlyOwner {}

    function deposit(uint256 amount) public {
        require(amount > 0, "zero amount");
    }
}
```
