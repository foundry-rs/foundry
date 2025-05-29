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

contract UnusedImport is IContract {
    using mySymbol for address;

    struct FooBar {
        symbol3 foo;
        myOtherSymbol bar;
    }

    symbol4 myVar;

    function foo(uint256 a, symbol5 b) public view {}
}
