library symbol0 {
    function isUsed(address) internal pure returns (bool) {
        return true;
    }
}

type symbol1 is uint128;
type symbol3 is bytes32;
type symbol4 is uint256;
type symbol5 is uint256;
type symbol2 is bool;
type symbolNotUsed is address;
type symbolNotUsed2 is address;
type symbolNotUsed3 is address;

contract BaseContract {}
interface IContract {
    function foo(uint256 a, uint256 b) external view returns (uint256);
    function convert(address addr) external pure returns (uint256);
}

interface IContractNotUsed {
    function doSomething() external;
}

interface docSymbol {}
interface docSymbol2 {}
interface docSymbolWrongTag {}

interface eventSymbol {
    event foo(uint256 bar);
}
