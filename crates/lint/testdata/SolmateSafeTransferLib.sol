//@compile-flags: --only-lint solmate-safe-transfer-lib
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {IToken, SafeTransferLib} from "./auxiliary/solmate/SolmateMocks.sol";
import {SafeTransferLib as STL, ITokenAux} from "./auxiliary/solmate/SolmateSafeTransferLib.sol";
import {SafeTransferLib as SoladySTL} from "./auxiliary/solady/SafeTransferLib.sol";
import {SafeTransferLib as FixedSTL, IToken as ITokenFixed} from "./auxiliary/solmate-fixed/SafeTransferLib.sol";

// Tests for `solmate-safe-transfer-lib`: the released solmate v6 `SafeTransferLib` treats a
// call that returns no data as a success without checking that the token has code, so a token
// operation against a token-less address is a silent no-op. A reference is flagged when it
// resolves to `safeTransfer` / `safeTransferFrom` / `safeApprove` declared in a library named
// exactly `SafeTransferLib` whose source comes from a solmate package path, whatever the call
// form. `safeTransferETH` involves no token code and stays clean, as do same-name functions
// declared in other libraries and same-name libraries from other packages (Solady).
// Note: this file's own path names solmate, so the canonical mirror and the out-of-package
// probes live in auxiliary files whose paths decide their provenance.

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

// Solady's SafeTransferLib shares the library and function names but checks token code, and
// its path does not name solmate: the provenance check keeps it out of scope.
contract UsesSolady {
    using SoladySTL for IToken;

    IToken internal token;

    function viaSolady(address to, uint256 amount) internal {
        token.safeTransfer(to, amount);
    }
}

// A same-name `SafeTransferLib` under a `solmate-fixed/` path: the string "solmate" is a
// substring of that path but not one of its components, so the provenance check must treat
// it as unrelated code and keep it out of scope.
contract UsesFixedSolmate {
    using FixedSTL for ITokenFixed;

    ITokenFixed internal token;

    function viaFixed(address to, uint256 amount) internal {
        token.safeTransfer(to, amount);
    }
}
