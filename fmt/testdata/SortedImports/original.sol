import "SomeFile3.sol";
import "SomeFile2.sol";
import "SomeFile1.sol" as SomeOtherFile;
import "SomeFile0.sol" as SomeOtherFile;

import "AnotherFile2.sol" as SomeSymbol;
import "AnotherFile1.sol" as SomeSymbol;

import {symbol2, symbol1 as alias} from "File3.sol";
import {symbol2, symbol1 as alias} from "File2.sol";
import {symbol1 as alias1, symbol2 as alias2, symbol3 as alias3, symbol4} from "File6.sol";
import {symbol3 as alias1, symbol2 as alias2, symbol1 as alias3, symbol4} from "File0.sol";

uint256 constant someConstant = 10;

import {Something3, Something2} from "someFile.sol";
