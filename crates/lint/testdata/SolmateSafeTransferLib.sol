//@compile-flags: --only-lint solmate-safe-transfer-lib
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {SafeTransferLib as STL, ITokenAux} from "./auxiliary/SolmateSafeTransferLib.sol";

// Tests for `solmate-safe-transfer-lib`: the released solmate v6 `SafeTransferLib` treats a
// call that returns no data as a success without checking that the token has code, so a token
// operation against a token-less address is a silent no-op. A reference is flagged when it
// resolves to `safeTransfer` / `safeTransferFrom` / `safeApprove` declared in a library named
// exactly `SafeTransferLib`, whatever the call form. `safeTransferETH` involves no token code
// and stays clean, as do same-name functions declared in other libraries.

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

// Minimal mirror of solmate's SafeTransferLib: the token operations next to the ETH one.
library SafeTransferLib {
    function safeTransferETH(address to, uint256 amount) internal {
        payable(to).transfer(amount);
    }

    function safeTransfer(IToken token, address to, uint256 amount) internal {
        token.transfer(to, amount);
    }

    function safeTransferFrom(IToken token, address from, address to, uint256 amount) internal {
        token.transferFrom(from, to, amount);
    }

    function safeApprove(IToken token, address spender, uint256 amount) internal {
        token.approve(spender, amount);
    }
}

// Same function names in another library (Uniswap's TransferHelper style): out of scope.
library TransferHelper {
    function safeTransfer(IToken token, address to, uint256 amount) internal {
        token.transfer(to, amount);
    }

    function safeTransferFrom(IToken token, address from, address to, uint256 amount) internal {
        token.transferFrom(from, to, amount);
    }
}

contract UsesSolmate {
    using SafeTransferLib for IToken;

    IToken internal token;

    function viaUsingFor(address to, uint256 amount) internal {
        token.safeTransfer(to, amount); //~WARN: Solmate's `SafeTransferLib` does not check
    }

    function viaQualified(address from, address to, uint256 amount) internal {
        SafeTransferLib.safeTransferFrom(token, from, to, amount); //~WARN: Solmate's `SafeTransferLib` does not check
    }

    function viaApprove(address spender, uint256 amount) internal {
        token.safeApprove(spender, amount); //~WARN: Solmate's `SafeTransferLib` does not check
    }

    function ethTransferIsClean(address to, uint256 amount) internal {
        SafeTransferLib.safeTransferETH(to, amount);
    }
}

contract UsesAliasedImport {
    ITokenAux internal token;

    // The import alias renames the call site, not the declared library the call resolves to.
    function viaAlias(address to, uint256 amount) internal {
        STL.safeTransfer(token, to, amount); //~WARN: Solmate's `SafeTransferLib` does not check
    }
}

// The same operations through Uniswap-style TransferHelper: out of scope.
contract UsesTransferHelper {
    using TransferHelper for IToken;

    IToken internal token;

    function viaHelper(address to, uint256 amount) internal {
        token.safeTransfer(to, amount);
    }

    function viaHelperQualified(address from, address to, uint256 amount) internal {
        TransferHelper.safeTransferFrom(token, from, to, amount);
    }
}

// The granular form binds a single function: it resolves to the same declaration.
contract GranularUsing {
    using {SafeTransferLib.safeTransfer} for IToken;

    IToken internal token;

    function viaGranular(address to, uint256 amount) internal {
        token.safeTransfer(to, amount); //~WARN: Solmate's `SafeTransferLib` does not check
    }
}

// Calls in a constructor or a modifier are calls like any other.
contract EagerPayer {
    using SafeTransferLib for IToken;

    IToken internal token;

    constructor(address to) {
        token.safeTransfer(to, 1); //~WARN: Solmate's `SafeTransferLib` does not check
    }

    modifier paying(address to, uint256 amount) {
        token.safeTransfer(to, amount); //~WARN: Solmate's `SafeTransferLib` does not check
        _;
    }

    function noop(address to, uint256 amount) external paying(to, amount) {}
}

// A free function is analyzed like a contract function.
function freePay(IToken token, address to, uint256 amount) {
    SafeTransferLib.safeTransfer(token, to, amount); //~WARN: Solmate's `SafeTransferLib` does not check
}

// A reference used as a value is a use of the unchecked operation too.
contract RefUser {
    function pick() internal pure returns (function(IToken, address, uint256) internal) {
        return SafeTransferLib.safeTransfer; //~WARN: Solmate's `SafeTransferLib` does not check
    }
}

// A contract's own `safeTransfer` is not the solmate library: out of scope.
contract NotALibrary {
    function safeTransfer(IToken token, address to, uint256 amount) internal {
        token.transfer(to, amount);
    }

    function viaContract(IToken token, address to, uint256 amount) internal {
        safeTransfer(token, to, amount);
    }
}
