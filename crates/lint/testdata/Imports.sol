//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {
    symbol0 as mySymbol,
    symbol1 as myOtherSymbol,
    symbol2 as notUsed, //~NOTE: unused imports should be removed
    symbol3,
    symbol4,
    symbol5,
    docSymbol,
    docSymbol2,
    docSymbolWrongTag, //~NOTE: unused imports should be removed
    eventSymbol,
    BaseContract,
    symbolNotUsed, //~NOTE: unused imports should be removed
    IContract,
    IContractNotUsed //~NOTE: unused imports should be removed
} from "./auxiliary/ImportsFile.sol";

// forge-lint: disable-next-item
import {
    symbolNotUsed2
} from "./auxiliary/ImportsFile.sol";

// in this case, disabling the following line doesn't do anything
// forge-lint: disable-next-line
import {
    symbolNotUsed3 //~NOTE: unused imports should be removed
} from "./auxiliary/ImportsFile.sol";

import {
    CONSTANT_0,
    CONSTANT_1 //~NOTE: unused imports should be removed
} from "./auxiliary/ImportsConstants.sol";

import {
    MyType,
    MyOtherType,
    YetAnotherType //~NOTE: unused imports should be removed
} from "./auxiliary/ImportsTypes.sol";

import "./auxiliary/ImportsSomeFile.sol"; //~NOTE: use named imports '{A, B}' or alias 'import ".." as X'
import "./auxiliary/ImportsAnotherFile.sol"; //~NOTE: use named imports '{A, B}' or alias 'import ".." as X'

import "./auxiliary/ImportsSomeFile2.sol" as SomeFile2;
import "./auxiliary/ImportsAnotherFile2.sol" as AnotherFile2; //~NOTE: unused imports should be removed

import * as Utils from "./auxiliary/ImportsUtils.sol";
import * as OtherUtils from "./auxiliary/ImportsUtils2.sol"; //~NOTE: unused imports should be removed


abstract contract UnusedImport is IContract, BaseContract {
    using mySymbol for address;

    uint256 constant MY_CONSTANT = CONSTANT_0;

    struct FooBar {
        symbol3 foo;
        myOtherSymbol bar;
    }

    /// @dev docSymbolWrongTag
    SomeFile.Baz public myStruct;
    SomeFile2.Baz public myStruct2;
    symbol4 public myVar;

    function foo(uint256 a, symbol5 b) external override(BaseContract) returns (uint256) {
        uint256 c = Utils.calculate(a, symbol5.unwrap(b));
        emit eventSymbol.foo(c);
        return c;
    }

    function convert(address addr) public pure returns (MyOtherType) {
        MyType a = MyType.wrap(123);
        return MyOtherType.wrap(MyType.unwrap(a));
    }
}
