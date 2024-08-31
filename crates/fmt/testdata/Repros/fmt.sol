// Repros of fmt issues

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
            (
                "bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla"
            )
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
        assembly{ sstore(   1,    1) /* inline comment*/ // forgefmt: disable-line
            sstore(2, 2)
        }
    }

    function test2() public {
        assembly{ sstore(   1,    1) // forgefmt: disable-line
            sstore(2, 2)
            sstore(3,    3)// forgefmt: disable-line
            sstore(4, 4)
        }
    }

    function test3() public {
        // forgefmt: disable-next-line
        assembly{ sstore(   1,    1)
            sstore(2, 2)
            sstore(3,    3)// forgefmt: disable-line
            sstore(4, 4)
        }// forgefmt: disable-line
    }

    function test4() public {
        // forgefmt: disable-next-line
                  assembly{
            sstore(1, 1)
            sstore(2, 2)
            sstore(3,    3)// forgefmt: disable-line
            sstore(4, 4)
        }// forgefmt: disable-line
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
