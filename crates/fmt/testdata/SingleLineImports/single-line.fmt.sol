// config: single_line_imports = true
// Test cases for single_line_imports configuration

// Single import that exceeds line length (121 chars)
import {ITransparentUpgradeableProxy} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";

// Single import with trailing comment
import {SomeContract} from "path/to/SomeContract.sol"; // This is a comment

// Single import with comment above
// This contract handles authentication
import {AuthManager} from "contracts/auth/AuthManager.sol";

// Multiple imports should still wrap regardless of config
import {
    Contract1,
    Contract2,
    Contract3,
    Contract4
} from "long/path/to/contracts.sol";

// Short single import
import {Token} from "Token.sol";

// Single import with alias
import {LongContractName as LCN} from "contracts/LongContractName.sol";

// Mixed comment styles
import {MixedComment} from "Mixed.sol"; /* block comment */ // line comment
