// config: style = "tab"
// Repros of fmt issues

// https://github.com/foundry-rs/foundry/issues/7944
import {ERC20} from "@contracts/token/ERC20/ERC20.sol";
import {ERC20Permit} from "@contracts/token/ERC20/ext/ERC20Permit.sol";
import {ERC20Burnable} from "@contracts/token/ERC20/ext/ERC20Burnable.sol";
import {IERC20} from "@contracts/token/ERC20/IERC20.sol";
import {IERC20Permit} from "@contracts/token/ERC20/ext/ERC20Permit.sol";
import {AccessControl} from "@contracts/access/AccessControl.sol";

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
		if (  true  ) {	// forgefmt: disable-line
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

	function test_binOpsInsideNestedBlocks() public {
		for (uint256 i = 0; i < steps.length; i++) {
			if (
				step.opcode == 0x52
					&& /*MSTORE*/ step.stack[0] == testContract.memPtr() // MSTORE offset
					&& step.stack[1] == testContract.expectedValueInMemory() // MSTORE val
			) {
				mstoreCalled = true;
			}
		}
	}
}

contract ERC1967Factory {
	/// @dev Returns a pointer to the initialization code of a proxy created via this factory.
	function _initCode() internal view returns (bytes32 m) {
		assembly {
			/**
			 *	-------------------------------------------------------------------------------------+
			 *	CREATION (9 bytes)                                                                   |
			 *	-------------------------------------------------------------------------------------|
			 *	Opcode     | Mnemonic        | Stack               | Memory                          |
			 *	-------------------------------------------------------------------------------------|
			 *	60 runSize | PUSH1 runSize   | r                   |                                 |
			 *	3d         | RETURNDATASIZE  | 0 r                 |                                 |
			 *	81         | DUP2            | r 0 r               |                                 |
			 *	60 offset  | PUSH1 offset    | o r 0 r             |                                 |
			 *	3d         | RETURNDATASIZE  | 0 o r 0 r           |                                 |
			 *	39         | CODECOPY        | 0 r                 | [0..runSize): runtime code      |
			 *	f3         | RETURN          |                     | [0..runSize): runtime code      |
			 *	-------------------------------------------------------------------------------------|
			 *	RUNTIME (127 bytes)                                                                  |
			 *	-------------------------------------------------------------------------------------|
			 *	Opcode      | Mnemonic       | Stack               | Memory                          |
			 *	-------------------------------------------------------------------------------------|
			 *																						|
			 *	::: keep some values in stack :::::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	3d          | RETURNDATASIZE | 0                   |                                 |
			 *	3d          | RETURNDATASIZE | 0 0                 |                                 |
			 *																						|
			 *	::: check if caller is factory ::::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	33          | CALLER         | c 0 0               |                                 |
			 *	73 factory  | PUSH20 factory | f c 0 0             |                                 |
			 *	14          | EQ             | isf 0 0             |                                 |
			 *	60 0x57     | PUSH1 0x57     | dest isf 0 0        |                                 |
			 *	57          | JUMPI          | 0 0                 |                                 |
			 *																						|
			 *	::: copy calldata to memory :::::::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	36          | CALLDATASIZE   | cds 0 0             |                                 |
			 *	3d          | RETURNDATASIZE | 0 cds 0 0           |                                 |
			 *	3d          | RETURNDATASIZE | 0 0 cds 0 0         |                                 |
			 *	37          | CALLDATACOPY   | 0 0                 | [0..calldatasize): calldata     |
			 *																						|
			 *	::: delegatecall to implementation ::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	36          | CALLDATASIZE   | cds 0 0             | [0..calldatasize): calldata     |
			 *	3d          | RETURNDATASIZE | 0 cds 0 0           | [0..calldatasize): calldata     |
			 *	7f slot     | PUSH32 slot    | s 0 cds 0 0         | [0..calldatasize): calldata     |
			 *	54          | SLOAD          | i 0 cds 0 0         | [0..calldatasize): calldata     |
			 *	5a          | GAS            | g i 0 cds 0 0       | [0..calldatasize): calldata     |
			 *	f4          | DELEGATECALL   | succ                | [0..calldatasize): calldata     |
			 *																						|
			 *	::: copy returndata to memory :::::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	3d          | RETURNDATASIZE | rds succ            | [0..calldatasize): calldata     |
			 *	60 0x00     | PUSH1 0x00     | 0 rds succ          | [0..calldatasize): calldata     |
			 *	80          | DUP1           | 0 0 rds succ        | [0..calldatasize): calldata     |
			 *	3e          | RETURNDATACOPY | succ                | [0..returndatasize): returndata |
			 *																						|
			 *	::: branch on delegatecall status :::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	60 0x52     | PUSH1 0x52     | dest succ           | [0..returndatasize): returndata |
			 *	57          | JUMPI          |                     | [0..returndatasize): returndata |
			 *																						|
			 *	::: delegatecall failed, revert :::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	3d          | RETURNDATASIZE | rds                 | [0..returndatasize): returndata |
			 *	60 0x00     | PUSH1 0x00     | 0 rds               | [0..returndatasize): returndata |
			 *	fd          | REVERT         |                     | [0..returndatasize): returndata |
			 *																						|
			 *	::: delegatecall succeeded, return ::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	5b          | JUMPDEST       |                     | [0..returndatasize): returndata |
			 *	3d          | RETURNDATASIZE | rds                 | [0..returndatasize): returndata |
			 *	60 0x00     | PUSH1 0x00     | 0 rds               | [0..returndatasize): returndata |
			 *	f3          | RETURN         |                     | [0..returndatasize): returndata |
			 *																						|
			 *	::: set new implementation (caller is factory) ::::::::::::::::::::::::::::::::::::: |
			 *	5b          | JUMPDEST       | 0 0                 |                                 |
			 *	3d          | RETURNDATASIZE | 0 0 0               |                                 |
			 *	35          | CALLDATALOAD   | impl 0 0            |                                 |
			 *	60 0x20     | PUSH1 0x20     | w impl 0 0          |                                 |
			 *	35          | CALLDATALOAD   | slot impl 0 0       |                                 |
			 *	55          | SSTORE         | 0 0                 |                                 |
			 *																						|
			 *	::: no extra calldata, return :::::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	60 0x40     | PUSH1 0x40     | 2w 0 0              |                                 |
			 *	80          | DUP1           | 2w 2w 0 0           |                                 |
			 *	36          | CALLDATASIZE   | cds 2w 2w 0 0       |                                 |
			 *	11          | GT             | gt 2w 0 0           |                                 |
			 *	15          | ISZERO         | lte 2w 0 0          |                                 |
			 *	60 0x52     | PUSH1 0x52     | dest lte 2w 0 0     |                                 |
			 *	57          | JUMPI          | 2w 0 0              |                                 |
			 *																						|
			 *	::: copy extra calldata to memory :::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	36          | CALLDATASIZE   | cds 2w 0 0          |                                 |
			 *	03          | SUB            | t 0 0               |                                 |
			 *	80          | DUP1           | t t 0 0             |                                 |
			 *	60 0x40     | PUSH1 0x40     | 2w t t 0 0          |                                 |
			 *	3d          | RETURNDATASIZE | 0 2w t t 0 0        |                                 |
			 *	37          | CALLDATACOPY   | t 0 0               | [0..t): extra calldata          |
			 *																						|
			 *	::: delegatecall to implementation ::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	3d          | RETURNDATASIZE | 0 t 0 0             | [0..t): extra calldata          |
			 *	3d          | RETURNDATASIZE | 0 0 t 0 0           | [0..t): extra calldata          |
			 *	35          | CALLDATALOAD   | i 0 t 0 0           | [0..t): extra calldata          |
			 *	5a          | GAS            | g i 0 t 0 0         | [0..t): extra calldata          |
			 *	f4          | DELEGATECALL   | succ                | [0..t): extra calldata          |
			 *																						|
			 *	::: copy returndata to memory :::::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	3d          | RETURNDATASIZE | rds succ            | [0..t): extra calldata          |
			 *	60 0x00     | PUSH1 0x00     | 0 rds succ          | [0..t): extra calldata          |
			 *	80          | DUP1           | 0 0 rds succ        | [0..t): extra calldata          |
			 *	3e          | RETURNDATACOPY | succ                | [0..returndatasize): returndata |
			 *																						|
			 *	::: branch on delegatecall status :::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	60 0x52     | PUSH1 0x52     | dest succ           | [0..returndatasize): returndata |
			 *	57          | JUMPI          |                     | [0..returndatasize): returndata |
			 *																						|
			 *	::: delegatecall failed, revert :::::::::::::::::::::::::::::::::::::::::::::::::::: |
			 *	3d          | RETURNDATASIZE | rds                 | [0..returndatasize): returndata |
			 *	60 0x00     | PUSH1 0x00     | 0 rds               | [0..returndatasize): returndata |
			 *	fd          | REVERT         |                     | [0..returndatasize): returndata |
			 *	-------------------------------------------------------------------------------------+
			 */
			m := mload(0x40)
			// forgefmt: disable-start
			switch shr(112, address())
			case 0 {
				// If the factory's address has six or more leading zero bytes.
				mstore(add(m, 0x75), 0x604c573d6000fd) // 7
				mstore(add(m, 0x6e), 0x3d3560203555604080361115604c5736038060403d373d3d355af43d6000803e) // 32
				mstore(add(m, 0x4e), 0x3735a920a3ca505d382bbc545af43d6000803e604c573d6000fd5b3d6000f35b) // 32
				mstore(add(m, 0x2e), 0x14605157363d3d37363d7f360894a13ba1a3210667c828492db98dca3e2076cc) // 32
				mstore(add(m, 0x0e), address()) // 14
				mstore(m, 0x60793d8160093d39f33d3d336d) // 9 + 4
			}
			default {
				mstore(add(m, 0x7b), 0x6052573d6000fd) // 7
				mstore(add(m, 0x74), 0x3d356020355560408036111560525736038060403d373d3d355af43d6000803e) // 32
				mstore(add(m, 0x54), 0x3735a920a3ca505d382bbc545af43d6000803e6052573d6000fd5b3d6000f35b) // 32
				mstore(add(m, 0x34), 0x14605757363d3d37363d7f360894a13ba1a3210667c828492db98dca3e2076cc) // 32
				mstore(add(m, 0x14), address()) // 20
				mstore(m, 0x607f3d8160093d39f33d3d3373) // 9 + 4
			}
			// forgefmt: disable-end
		}
	}
}

/// @title Wrapped Ether Hook
/// @notice Hook for wrapping/unwrapping ETH in Uniswap V4 pools
/// @dev Implements 1:1 wrapping/unwrapping of ETH to WETH
contract WETHHook is BaseTokenWrapperHook {
	/// @notice The WETH9 contract
	WETH public immutable weth;

	/// @notice Creates a new WETH wrapper hook
	/// @param _manager The Uniswap V4 pool manager
	/// @param _weth The WETH9 contract address
	constructor(IPoolManager _manager, address payable _weth)
		BaseTokenWrapperHook(
			_manager,
			Currency.wrap(_weth), // wrapper token is WETH
			CurrencyLibrary.ADDRESS_ZERO // underlying token is ETH (address(0))
		)
	{
		weth = WETH(payable(_weth));
	}
}
