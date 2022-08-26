pragma solidity >0.8.13;

import "ds-test/test.sol";

contract Contract {
    address public addr = address(0xbeef);
    string public str = "hello";
    uint256 public num = 1337;
    uint256 public pushNum;

    function changeAddress(address _addr) public {
        if (_addr == addr) {
            addr = address(0);
        }
    }

    function changeString(string memory _str) public {
        if (keccak256(bytes(_str)) == keccak256(bytes(str))) {
            str = "";
        }
    }

    function changeUint(uint256 _num) public {
        if (_num == num) {
            num = 0;
        }
    }

    function push(uint256 _num) public {
        if (_num == 68) {
            pushNum = 69;
        }
    }
}

contract InvariantStorageTest is DSTest {
    Contract c;

    function setUp() public {
        c = new Contract();
    }

    function invariantChangeAddress() public {
        require(c.addr() == address(0xbeef), "changedAddr");
    }

    function invariantChangeString() public {
        require(keccak256(bytes(c.str())) == keccak256(bytes("hello")), "changedStr");
    }

    function invariantChangeUint() public {
        require(c.num() == 1337, "changedUint");
    }

    function invariantPush() public {
        require(c.pushNum() == 0, "pushUint");
    }
}
