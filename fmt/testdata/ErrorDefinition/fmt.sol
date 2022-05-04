pragma solidity ^0.8.4;

error TopLevelCustomError();
error TopLevelCustomErrorWithArg(uint256 x);
error TopLevelCustomErrorArgWithoutName(string);

contract Errors {
    error ContractCustomError();
    error ContractCustomErrorWithArg(uint256 x);
    error ContractCustomErrorArgWithoutName(string);
}
