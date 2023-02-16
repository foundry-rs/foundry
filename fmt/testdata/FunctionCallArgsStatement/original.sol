interface ITarget {
    function run() external payable;
    function veryAndVeryLongNameOfSomeRunFunction() external payable;
}

contract FunctionCallArgsStatement {
    ITarget public target;

    function estimate() public returns (uint256 gas) {
        gas = 1 gwei;
    }

    function veryAndVeryLongNameOfSomeGasEstimateFunction() public returns (uint256) {
        return gasleft();
    }

    function value(uint256 val) public returns (uint256) {
        return val;
    }

    function test() external {
        target.run{ gas: gasleft(), value: 1 wei };

        target.run{gas:1,value:0x00}();

        target.run{ 
                gas : 1000, 
        value: 1 ether 
        } ();

        target.run{  gas: estimate(),
    value: value(1) }(); 

        target.run { value:
        value(1 ether), gas: veryAndVeryLongNameOfSomeGasEstimateFunction() } ();

        target.run /* comment 1 */ { value: /* comment2 */ 1 }; 

        target.run { /* comment3 */ value: 1, // comment4
        gas: gasleft()};

        target.run {
            // comment5
            value: 1,
            // comment6
            gas: gasleft()};

        vm.expectEmit({ checkTopic1: false, checkTopic2: false    });
    }
}