// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract ScreamingSnakeCaseTest {
    uint256 constant _SCREAMING_SNAKE_CASE = 0;
    uint256 constant SCREAMING_SNAKE_CASE = 0;
    uint256 constant SCREAMINGSNAKECASE = 0;

    uint256 constant screamingSnakeCase = 0; //~NOTE: constants should use SCREAMING_SNAKE_CASE
    uint256 constant screaming_snake_case = 0; //~NOTE: constants should use SCREAMING_SNAKE_CASE
    uint256 constant ScreamingSnakeCase = 0; //~NOTE: constants should use SCREAMING_SNAKE_CASE
    uint256 constant SCREAMING_snake_case = 0; //~NOTE: constants should use SCREAMING_SNAKE_CASE

    uint256 immutable _SCREAMING_SNAKE_CASE_1 = 0;
    uint256 immutable SCREAMING_SNAKE_CASE_1 = 0;
    uint256 immutable SCREAMINGSNAKECASE0 = 0;
    uint256 immutable SCREAMINGSNAKECASE_ = 0;

    uint256 immutable screamingSnakeCase0 = 0; //~NOTE: immutables should use SCREAMING_SNAKE_CASE
    uint256 immutable screaming_snake_case0 = 0; //~NOTE: immutables should use SCREAMING_SNAKE_CASE
    uint256 immutable ScreamingSnakeCase0 = 0; //~NOTE: immutables should use SCREAMING_SNAKE_CASE
    uint256 immutable SCREAMING_snake_case_0 = 0; //~NOTE: immutables should use SCREAMING_SNAKE_CASE
}
