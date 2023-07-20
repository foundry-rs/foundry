// config: sort_imports = true
import "AnotherFile1.sol" as SomeSymbol;
import "AnotherFile2.sol" as SomeSymbol;

import {symbol3 as alias1, symbol2 as alias2, symbol1 as alias3, symbol4} from "File0.sol";
import {symbol1 as alias, symbol2} from "File2.sol";
import {symbol1 as alias, symbol2} from "File3.sol";

import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from "File6.sol";
import "SomeFile0.sol" as SomeOtherFile;
import "SomeFile1.sol" as SomeOtherFile;
import "SomeFile2.sol";
import "SomeFile3.sol";

uint256 constant someConstant = 10;

import {Something2, Something3} from "someFile.sol";
