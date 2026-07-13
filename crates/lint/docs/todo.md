# TODO/FIXME comments

**Severity**: `Info`
**ID**: `todo-comment`

Flags `TODO` and `FIXME` markers left in comments, which signal unfinished work or known
bugs that have not been resolved before the code reached production.

## What it does

Scans every comment in the source file (single-line `//`, block `/* */`, and NatSpec `///`)
and reports comments containing a `TODO` or `FIXME` marker.

A marker is recognized when it appears at the **start of a whitespace-delimited token** and is **immediately followed by one of** `:` `(` `,` `;` `.` `)`.
Matching is case-insensitive, so `todo:`, `ToDo:`, and `FixMe:` all match.

## Why is this bad?

`TODO` and `FIXME` comments are development notes. Shipping them into production contracts signals incomplete work.

## Example

### Bad

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

### Good

```solidity
contract Vault {
    function withdraw() public onlyOwner {}

    function deposit(uint256 amount) public {
        require(amount > 0, "zero amount");
    }
}
```
