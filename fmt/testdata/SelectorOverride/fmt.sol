// https://github.com/hyperledger/solang/blob/b867c8a6c7a1ee89d405993abef458fc59e9b0fb/tests/contract_testcases/ewasm/selector_override.sol
contract SelectorOverride {
    constructor() selector=hex"abcd" {}

    modifier m() selector=hex"" {
        _;
    }

    receive() external payable selector=hex"1" {}
    fallback() external selector=hex"abc" {}
    function i() internal selector=hex"abdd" {}
    function p() private selector=hex"abdd" {}
}
