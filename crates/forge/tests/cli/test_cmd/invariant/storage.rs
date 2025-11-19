use super::*;

forgetest_init!(storage, |prj, cmd| {
    prj.add_test(
        "name",
        r#"
import "forge-std/Test.sol";

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

contract InvariantStorageTest is Test {
    Contract c;

    function setUp() public {
        c = new Contract();
    }

    function invariantChangeAddress() public view {
        require(c.addr() == address(0xbeef), "changedAddr");
    }

    function invariantChangeString() public view {
        require(keccak256(bytes(c.str())) == keccak256(bytes("hello")), "changedStr");
    }

    function invariantChangeUint() public view {
        require(c.num() == 1337, "changedUint");
    }

    function invariantPush() public view {
        require(c.pushNum() == 0, "pushUint");
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Suite result: FAILED. 0 passed; 4 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 4 failed, 0 skipped (4 total tests)

Failing tests:
Encountered 4 failing tests in test/name.sol:InvariantStorageTest
...
"#]]);
});
