// deps: ImportsFile.sol, ImportsConstants.sol, ImportsTypes.sol, ImportsSomeFile.sol, ImportsSomeFile2.sol, ImportsAnotherFile.sol, ImportsAnotherFile2.sol, ImportsUtils.sol, ImportsUtils2.sol

import {
    symbol0 as mySymbol,
    symbol1 as myOtherSymbol,
    symbol2 as notUsed, //~NOTE: unused imports should be removed
    symbol3,
    symbol4,
    symbol5,
    symbolNotUsed, //~NOTE: unused imports should be removed
    IContract,
    IContractNotUsed //~NOTE: unused imports should be removed
} from "ImportsFile.sol";

import {
    CONSTANT_0,
    CONSTANT_1 //~NOTE: unused imports should be removed
} from "ImportsConstants.sol";

import {
    MyType,
    MyOtherType,
    YetAnotherType //~NOTE: unused imports should be removed
} from "ImportsTypes.sol";

import "ImportsSomeFile.sol"; //~NOTE: use named imports '{A, B}' or alias 'import ".." as X'
import "ImportsAnotherFile.sol"; //~NOTE: use named imports '{A, B}' or alias 'import ".." as X'

import "ImportsSomeFile2.sol" as SomeFile2;
import "ImportsAnotherFile2.sol" as AnotherFile2; //~NOTE: unused imports should be removed

import * as Utils from "ImportsUtils.sol";
import * as OtherUtils from "ImportsUtils2.sol"; //~NOTE: unused imports should be removed


contract UnusedImport is IContract {
    using mySymbol for address;

    uint256 constant MY_CONSTANT = CONSTANT_0;

    struct FooBar {
        symbol3 foo;
        myOtherSymbol bar;
    }

    SomeFile.Baz public myStruct;
    SomeFile2.Baz public myStruct2;
    symbol4 public myVar;

    function foo(uint256 a, symbol5 b) public view returns (uint256) {
        uint256 c = Utils.calculate(a, b);
        return c;
    }

    function convert(address addr) public pure returns (MyOtherType) {
        MyType a = MyType.wrap(123);
        return MyOtherType.wrap(a);
    }
}
