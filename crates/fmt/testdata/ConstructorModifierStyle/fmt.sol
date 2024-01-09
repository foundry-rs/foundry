// SPDX-License-Identifier: MIT

pragma solidity ^0.5.2;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {ERC1155} from "solmate/tokens/ERC1155.sol";

import {IAchievements} from "./interfaces/IAchievements.sol";
import {SoulBound1155} from "./abstracts/SoulBound1155.sol";

contract Achievements is IAchievements, SoulBound1155, Ownable {
    constructor(address owner) Ownable() ERC1155() {}
}
