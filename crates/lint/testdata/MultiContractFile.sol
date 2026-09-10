//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract A {}

contract B {} //~NOTE: file contains multiple contracts, interfaces or libraries

contract C {}

interface I {}

library L {}
