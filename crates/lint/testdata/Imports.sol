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
} from "File.sol";

import {
    CONSTANT_0,
    CONSTANT_1 //~NOTE: unused imports should be removed
} from "Constants.sol";

import {
    MyTpe,
    MyOtherType,
    YetAnotherType //~NOTE: unused imports should be removed
} from "Types.sol";

import "SomeFile.sol"; //~NOTE: use named imports '{A, B}' or alias 'import ".." as X'
import "AnotherFile.sol"; //~NOTE: use named imports '{A, B}' or alias 'import ".." as X'

import "some_file_2.sol" as SomeFile2;
import "another_file_2.sol" as AnotherFile2; //~NOTE: unused imports should be removed

import * as Utils from "utils.sol";
import * as OtherUtils from "utils2.sol"; //~NOTE: unused imports should be removed


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
        MyType a = MyTpe.wrap(123);
        return MyOtherType.wrap(a);
    }
}
