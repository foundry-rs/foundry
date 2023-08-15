pragma solidity ^0.5.2;

contract Events {
    event Event1();
    event Event1() anonymous;

    event Event1(uint256);
    event Event1(uint256) anonymous;

    event Event1(uint256 a);
    event Event1(uint256 a) anonymous;

    event Event1(uint256 indexed);
    event Event1(uint256 indexed) anonymous;

    event Event1(uint256 indexed a);
    event Event1(uint256 indexed a) anonymous;

    event Event1(uint256, uint256, uint256, uint256, uint256, uint256, uint256);
    event Event1(uint256, uint256, uint256, uint256, uint256, uint256, uint256) anonymous;

    event Event1(uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256);
    event Event1(uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256) anonymous;

    event Event1(uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256);
    event Event1(uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256, uint256) anonymous;

    event Event1(uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a);
    event Event1(uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a, uint256 a) anonymous;

    event Event1(uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed);
    event Event1(uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed, uint256 indexed) anonymous;

    event Event1(uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a);
    event Event1(uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a, uint256 indexed a) anonymous;
}
