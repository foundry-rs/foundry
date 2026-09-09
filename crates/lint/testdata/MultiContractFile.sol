//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract A {}

contract B {} //~NOTE: prefer having only one contract, interface or library per file

contract C {}

interface I {}

library L {}
