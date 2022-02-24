pragma solidity ^0.7.6;

interface ERC20 {
    function balanceOf(address) external view returns (uint256);
    function deposit() payable external;
}

interface VM {
    function startPrank(address) external;
}

library T {
    function getBal(ERC20 t, address who) public view returns (uint256) {
        return t.balanceOf(who);
    }
}

contract C {
    ERC20 weth = ERC20(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2);
    VM constant vm  = VM(address(bytes20(uint160(uint256(keccak256('hevm cheat code'))))));
    address who = 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045;

    event log_uint(uint256);

    function run() external {
        // impersonate the account
        vm.startPrank(who);

        uint256 balanceBefore = T.getBal(weth, who);
        emit log_uint(balanceBefore);

        weth.deposit{value: 15 ether}();

        uint256 balanceAfter = weth.balanceOf(who);
        emit log_uint(balanceAfter);

    }
}