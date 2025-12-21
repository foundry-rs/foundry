// config: bracket_spacing = true
import "SomeFile.sol";
import "SomeFile.sol";
import "SomeFile.sol" as SomeOtherFile;
import "SomeFile.sol" as SomeOtherFile;
import "AnotherFile.sol" as SomeSymbol;
import "AnotherFile.sol" as SomeSymbol;
import { symbol1 as alias0, symbol2 } from "File.sol";
import { symbol1 as alias0, symbol2 } from "File.sol";
import {
    symbol1 as alias1,
    symbol2 as alias2,
    symbol3 as alias3,
    symbol4
} from "File2.sol";
import {
    symbol1 as alias1,
    symbol2 as alias2,
    symbol3 as alias3,
    symbol4
} from "File2.sol";

// Single import that exceeds line length (121 chars)
import {
    ITransparentUpgradeableProxy
} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
