// config: sort_imports = true
// Repros of fmt issues

// https://github.com/foundry-rs/foundry/issues/7944
import {AccessControl} from "@contracts/access/AccessControl.sol";
import {ERC20} from "@contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@contracts/token/ERC20/IERC20.sol";
import {ERC20Burnable} from "@contracts/token/ERC20/ext/ERC20Burnable.sol";
import {ERC20Permit} from "@contracts/token/ERC20/ext/ERC20Permit.sol";
import {IERC20Permit} from "@contracts/token/ERC20/ext/ERC20Permit.sol";

// https://github.com/foundry-rs/foundry/issues/4403
function errorIdentifier() {
    bytes memory error = bytes("");
    if (error.length > 0) {}
}

// https://github.com/foundry-rs/foundry/issues/7549
function one() external {
    this.other({
        data: abi.encodeCall(
            this.other,
            ("bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla")
        )
    });
}

// https://github.com/foundry-rs/foundry/issues/3979
contract Format {
    bool public test;

    function testing(uint256 amount) public payable {
        if (
            // This is a comment
            msg.value == amount
        ) {
            test = true;
        } else {
            test = false;
        }

        if (
            // Another one
            block.timestamp >= amount
        ) {}
    }
}

// https://github.com/foundry-rs/foundry/issues/3830
contract TestContract {
    function test(uint256 a) public {
        if (a > 1) {
            a = 2;
        } // forgefmt: disable-line
    }

    function test1() public {
        assembly { sstore(   1,    1) /* inline comment*/ // forgefmt: disable-line
            sstore(2, 2)
        }
    }

    function test2() public {
        assembly { sstore(   1,    1) // forgefmt: disable-line
            sstore(2, 2)
            sstore(3,    3) // forgefmt: disable-line
            sstore(4, 4)
        }
    }

    function test3() public {
        // forgefmt: disable-next-line
        assembly{ sstore(   1,    1)
            sstore(2, 2)
            sstore(3,    3) // forgefmt: disable-line
            sstore(4, 4)
        } // forgefmt: disable-line
    }

    function test4() public {
        // forgefmt: disable-next-line
                  assembly {
            sstore(1, 1)
            sstore(2, 2)
            sstore(3,    3) // forgefmt: disable-line
            sstore(4, 4)
        } // forgefmt: disable-line
        if (condition) execute(); // comment7
    }

    function test5() public {
        assembly { sstore(0, 0) }// forgefmt: disable-line
    }

    function test6() returns (bool) { // forgefmt: disable-line
        if (  true  ) {  // forgefmt: disable-line
        }
        return true ;  }  // forgefmt: disable-line

    function test7() returns (bool) { // forgefmt: disable-line
        if (true) {  // forgefmt: disable-line
            uint256 a     =     1; // forgefmt: disable-line
        }
        return true;
    }

    function test8() returns (bool) { // forgefmt: disable-line
        if (  true  ) {    // forgefmt: disable-line
            uint256 a = 1;
        } else {
            uint256 b     =     1; // forgefmt: disable-line
        }
        return true;
    }
}

// https://github.com/foundry-rs/foundry/issues/5825
library MyLib {
    bytes32 private constant TYPE_HASH = keccak256(
        // forgefmt: disable-start
        "MyStruct("
            "uint8 myEnum,"
                "address myAddress"
                    ")"
        // forgefmt: disable-end
    );

    bytes32 private constant TYPE_HASH_1 = keccak256(
        "MyStruct("    "uint8 myEnum,"    "address myAddress"    ")" // forgefmt: disable-line
    );

    // forgefmt: disable-start
    bytes32 private constant TYPE_HASH_2 = keccak256(
        "MyStruct("
            "uint8 myEnum,"
            "address myAddress"
        ")"
    );
    // forgefmt: disable-end
}

contract IfElseTest {
    function setNumber(uint256 newNumber) public {
        number = newNumber;
        if (newNumber = 1) {
            number = 1;
        } else if (newNumber = 2) {
            //            number = 2;
        } else {
            newNumber = 3;
        }
    }
}

contract DbgFmtTest is Test {
    function test_argsList() public {
        uint256 result1 = internalNoArgs({});
        result2 = add({a: 1, b: 2});
    }

    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function internalNoArgs() internal pure returns (uint256) {
        return 0;
    }
}

// https://github.com/foundry-rs/foundry/issues/11249
function argListRepro(address tokenIn, uint256 amountIn, bool data) {
    maverickV2SwapCallback(
        tokenIn,
        amountIn, // forgefmt: disable-line
        // forgefmt: disable-next-line
        0,/* we didn't bother loading `amountOut` because we don't use it */
        data
    );
}

contract NestedCallsTest is Test {
    string constant errMsg = "User provided message";
    uint256 constant maxDecimals = 77;

    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_nestedCalls() public {
        vm._expectCheatcodeRevert(
            bytes(string.concat(errMsg, ": ", left, " != ", right))
        );
    }

    function test_assemblyFnComments() public {
        assembly {
            function setJPoint(i, x, y, z) {
                // We will multiply by `0x80` (i.e. `shl(7, i)`) instead
                // since the memory expansion costs are cheaper than doing `mul(0x60, i)`.
                // Also help combine the lookup expression for `u1` and `u2` in `jMultShamir`.
                i := shl(7, i)
                mstore(i, x)
                mstore(add(i, returndatasize()), y)
                mstore(add(i, 0x40), z)
            }
        }
    }

    function test_longCall() public {
        uint256 fork =
            vm.createSelectFork("polygon", bytes32(0xdeadc0ffeedeadbeef));

        vm._expectCheatcodeRevert("short msg doesn't break");
        vm._expectCheatcodeRevert(
            "failed parsing as `uint256`: missing hex prefix for hex string"
        );

        bytes4[] memory targets = new bytes4[](0);
        targets[0] =
            FuzzArtifactSelector("TargetArtifactSelectors.t.sol:Hi", selectors);

        ConstructorVictim victim = new ConstructorVictim(
            sender, "msg.sender", "not set during prank"
        );
    }
}
