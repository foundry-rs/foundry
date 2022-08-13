pragma solidity ^0.8.4;

error
  TopLevelCustomError();
  error TopLevelCustomErrorWithArg(uint    x)  ;
error TopLevelCustomErrorArgWithoutName  (string);

contract Errors {
  error
    ContractCustomError();
    error ContractCustomErrorWithArg(uint    x)  ;
  error ContractCustomErrorArgWithoutName  (string);
}