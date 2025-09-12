// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title SimpleRevert - Basic revert testing contract
contract SimpleRevert {
    function doRevert(string memory reason) public pure {
        revert(reason);
    }

    function doRequire(uint256 value) public pure {
        require(value > 0, "Value must be greater than zero");
    }

    function doAssert() public pure {
        assert(false);
    }

    error CustomError(uint256 code, address sender);

    function doCustomError() public view {
        revert CustomError(42, msg.sender);
    }
}
