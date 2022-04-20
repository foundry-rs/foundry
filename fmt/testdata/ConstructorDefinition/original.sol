pragma solidity ^0.5.2;

contract Constructors is Ownable, Changeable {
    function Constructors(variable1) public Changeable(variable1) Ownable() onlyOwner {
    }

    constructor(variable1, variable2, variable3, variable4, variable5, variable6, variable7) public Changeable(variable1, variable2, variable3, variable4, variable5, variable6, variable7) Ownable() onlyOwner {}
}