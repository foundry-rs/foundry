/**
 * Submitted for verification at Etherscan.io on 2021-09-16
 */

// SPDX-License-Identifier: AGPL-3.0-or-later
//
// DssExecLib.sol -- MakerDAO Executive Spellcrafting Library
//
// Copyright (C) 2020 Maker Ecosystem Growth Holdings, Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
pragma solidity >=0.6.12 <0.7.0;

pragma experimental ABIEncoderV2;

struct CollateralOpts {
    bytes32 ilk;
    address gem;
    address join;
    address clip;
    address calc;
    address pip;
    bool isLiquidatable;
    bool isOSM;
    bool whitelistOSM;
    uint256 ilkDebtCeiling;
    uint256 minVaultAmount;
    uint256 maxLiquidationAmount;
    uint256 liquidationPenalty;
    uint256 ilkStabilityFee;
    uint256 startingPriceFactor;
    uint256 breakerTolerance;
    uint256 auctionDuration;
    uint256 permittedDrop;
    uint256 liquidationRatio;
    uint256 kprFlatReward;
    uint256 kprPctReward;
}

interface Initializable {
    function init(bytes32) external;
}

interface Authorizable {
    function rely(address) external;
    function deny(address) external;
}

interface Fileable {
    function file(bytes32, address) external;
    function file(bytes32, uint256) external;
    function file(bytes32, bytes32, uint256) external;
    function file(bytes32, bytes32, address) external;
}

interface Drippable {
    function drip() external returns (uint256);
    function drip(bytes32) external returns (uint256);
}

interface Pricing {
    function poke(bytes32) external;
}

interface ERC20 {
    function decimals() external returns (uint8);
}

interface DssVat {
    function hope(address) external;
    function nope(address) external;
    function ilks(bytes32) external returns (uint256 Art, uint256 rate, uint256 spot, uint256 line, uint256 dust);
    function Line() external view returns (uint256);
    function suck(address, address, uint256) external;
}

interface ClipLike {
    function vat() external returns (address);
    function dog() external returns (address);
    function spotter() external view returns (address);
    function calc() external view returns (address);
    function ilk() external returns (bytes32);
}

interface JoinLike {
    function vat() external returns (address);
    function ilk() external returns (bytes32);
    function gem() external returns (address);
    function dec() external returns (uint256);
    function join(address, uint256) external;
    function exit(address, uint256) external;
}

// Includes Median and OSM functions
interface OracleLike {
    function src() external view returns (address);
    function lift(address[] calldata) external;
    function drop(address[] calldata) external;
    function setBar(uint256) external;
    function kiss(address) external;
    function diss(address) external;
    function kiss(address[] calldata) external;
    function diss(address[] calldata) external;
    function orb0() external view returns (address);
    function orb1() external view returns (address);
}

interface MomLike {
    function setOsm(bytes32, address) external;
    function setPriceTolerance(address, uint256) external;
}

interface RegistryLike {
    function add(address) external;
    function xlip(bytes32) external view returns (address);
}

// https://github.com/makerdao/dss-chain-log
interface ChainlogLike {
    function setVersion(string calldata) external;
    function setIPFS(string calldata) external;
    function setSha256sum(string calldata) external;
    function getAddress(bytes32) external view returns (address);
    function setAddress(bytes32, address) external;
    function removeAddress(bytes32) external;
}

interface IAMLike {
    function ilks(bytes32) external view returns (uint256, uint256, uint48, uint48, uint48);
    function setIlk(bytes32, uint256, uint256, uint256) external;
    function remIlk(bytes32) external;
    function exec(bytes32) external returns (uint256);
}

interface LerpFactoryLike {
    function newLerp(
        bytes32 name_,
        address target_,
        bytes32 what_,
        uint256 startTime_,
        uint256 start_,
        uint256 end_,
        uint256 duration_
    ) external returns (address);
    function newIlkLerp(
        bytes32 name_,
        address target_,
        bytes32 ilk_,
        bytes32 what_,
        uint256 startTime_,
        uint256 start_,
        uint256 end_,
        uint256 duration_
    ) external returns (address);
}

interface LerpLike {
    function tick() external;
}

library DssExecLib {
    /**
     *
     */
    /**
     * Constants **
     */
    /**
     *
     */
    address public constant LOG = 0xdA0Ab1e0017DEbCd72Be8599041a2aa3bA7e740F;

    uint256 internal constant WAD = 10 ** 18;
    uint256 internal constant RAY = 10 ** 27;
    uint256 internal constant RAD = 10 ** 45;
    uint256 internal constant THOUSAND = 10 ** 3;
    uint256 internal constant MILLION = 10 ** 6;

    uint256 internal constant BPS_ONE_PCT = 100;
    uint256 internal constant BPS_ONE_HUNDRED_PCT = 100 * BPS_ONE_PCT;
    uint256 internal constant RATES_ONE_HUNDRED_PCT = 1000000021979553151239153027;

    /**
     *
     */
    /**
     * Math Functions **
     */
    /**
     *
     */
    function add(uint256 x, uint256 y) internal pure returns (uint256 z) {
        require((z = x + y) >= x);
    }

    function sub(uint256 x, uint256 y) internal pure returns (uint256 z) {
        require((z = x - y) <= x);
    }

    function mul(uint256 x, uint256 y) internal pure returns (uint256 z) {
        require(y == 0 || (z = x * y) / y == x);
    }

    function wmul(uint256 x, uint256 y) internal pure returns (uint256 z) {
        z = add(mul(x, y), WAD / 2) / WAD;
    }

    function rmul(uint256 x, uint256 y) internal pure returns (uint256 z) {
        z = add(mul(x, y), RAY / 2) / RAY;
    }

    function wdiv(uint256 x, uint256 y) internal pure returns (uint256 z) {
        z = add(mul(x, WAD), y / 2) / y;
    }

    function rdiv(uint256 x, uint256 y) internal pure returns (uint256 z) {
        z = add(mul(x, RAY), y / 2) / y;
    }

    /**
     *
     */
    /**
     * Core Address Helpers **
     */
    /**
     *
     */
    function dai() public view returns (address) {
        return getChangelogAddress("MCD_DAI");
    }

    function mkr() public view returns (address) {
        return getChangelogAddress("MCD_GOV");
    }

    function vat() public view returns (address) {
        return getChangelogAddress("MCD_VAT");
    }

    function cat() public view returns (address) {
        return getChangelogAddress("MCD_CAT");
    }

    function dog() public view returns (address) {
        return getChangelogAddress("MCD_DOG");
    }

    function jug() public view returns (address) {
        return getChangelogAddress("MCD_JUG");
    }

    function pot() public view returns (address) {
        return getChangelogAddress("MCD_POT");
    }

    function vow() public view returns (address) {
        return getChangelogAddress("MCD_VOW");
    }

    function end() public view returns (address) {
        return getChangelogAddress("MCD_END");
    }

    function esm() public view returns (address) {
        return getChangelogAddress("MCD_ESM");
    }

    function reg() public view returns (address) {
        return getChangelogAddress("ILK_REGISTRY");
    }

    function spotter() public view returns (address) {
        return getChangelogAddress("MCD_SPOT");
    }

    function flap() public view returns (address) {
        return getChangelogAddress("MCD_FLAP");
    }

    function flop() public view returns (address) {
        return getChangelogAddress("MCD_FLOP");
    }

    function osmMom() public view returns (address) {
        return getChangelogAddress("OSM_MOM");
    }

    function govGuard() public view returns (address) {
        return getChangelogAddress("GOV_GUARD");
    }

    function flipperMom() public view returns (address) {
        return getChangelogAddress("FLIPPER_MOM");
    }

    function clipperMom() public view returns (address) {
        return getChangelogAddress("CLIPPER_MOM");
    }

    function pauseProxy() public view returns (address) {
        return getChangelogAddress("MCD_PAUSE_PROXY");
    }

    function autoLine() public view returns (address) {
        return getChangelogAddress("MCD_IAM_AUTO_LINE");
    }

    function daiJoin() public view returns (address) {
        return getChangelogAddress("MCD_JOIN_DAI");
    }

    function lerpFab() public view returns (address) {
        return getChangelogAddress("LERP_FAB");
    }

    function clip(bytes32 _ilk) public view returns (address _clip) {
        _clip = RegistryLike(reg()).xlip(_ilk);
    }

    function flip(bytes32 _ilk) public view returns (address _flip) {
        _flip = RegistryLike(reg()).xlip(_ilk);
    }

    function calc(bytes32 _ilk) public view returns (address _calc) {
        _calc = ClipLike(clip(_ilk)).calc();
    }

    function getChangelogAddress(bytes32 _key) public view returns (address) {
        return ChainlogLike(LOG).getAddress(_key);
    }

    /**
     *
     */
    /**
     * Changelog Management **
     */
    /**
     *
     */
    /**
     * @dev Set an address in the MCD on-chain changelog.
     * @param _key Access key for the address (e.g. "MCD_VAT")
     * @param _val The address associated with the _key
     */
    function setChangelogAddress(bytes32 _key, address _val) public {
        ChainlogLike(LOG).setAddress(_key, _val);
    }

    /**
     * @dev Set version in the MCD on-chain changelog.
     * @param _version Changelog version (e.g. "1.1.2")
     */
    function setChangelogVersion(string memory _version) public {
        ChainlogLike(LOG).setVersion(_version);
    }
    /**
     * @dev Set IPFS hash of IPFS changelog in MCD on-chain changelog.
     * @param _ipfsHash IPFS hash (e.g. "QmefQMseb3AiTapiAKKexdKHig8wroKuZbmLtPLv4u2YwW")
     */

    function setChangelogIPFS(string memory _ipfsHash) public {
        ChainlogLike(LOG).setIPFS(_ipfsHash);
    }
    /**
     * @dev Set SHA256 hash in MCD on-chain changelog.
     * @param _SHA256Sum SHA256 hash (e.g. "e42dc9d043a57705f3f097099e6b2de4230bca9a020c797508da079f9079e35b")
     */

    function setChangelogSHA256(string memory _SHA256Sum) public {
        ChainlogLike(LOG).setSha256sum(_SHA256Sum);
    }

    /**
     *
     */
    /**
     * Authorizations **
     */
    /**
     *
     */
    /**
     * @dev Give an address authorization to perform auth actions on the contract.
     * @param _base   The address of the contract where the authorization will be set
     * @param _ward   Address to be authorized
     */
    function authorize(address _base, address _ward) public {
        Authorizable(_base).rely(_ward);
    }
    /**
     * @dev Revoke contract authorization from an address.
     * @param _base   The address of the contract where the authorization will be revoked
     * @param _ward   Address to be deauthorized
     */

    function deauthorize(address _base, address _ward) public {
        Authorizable(_base).deny(_ward);
    }
    /**
     * @dev Delegate vat authority to the specified address.
     * @param _usr Address to be authorized
     */

    function delegateVat(address _usr) public {
        DssVat(vat()).hope(_usr);
    }
    /**
     * @dev Revoke vat authority to the specified address.
     * @param _usr Address to be deauthorized
     */

    function undelegateVat(address _usr) public {
        DssVat(vat()).nope(_usr);
    }

    /**
     *
     */
    /**
     * OfficeHours Management **
     */
    /**
     *
     */

    /**
     * @dev Returns true if a time is within office hours range
     * @param _ts           The timestamp to check, usually block.timestamp
     * @param _officeHours  true if office hours is enabled.
     * @return              true if time is in castable range
     */
    function canCast(uint40 _ts, bool _officeHours) public pure returns (bool) {
        if (_officeHours) {
            uint256 day = (_ts / 1 days + 3) % 7;
            if (day >= 5) {
                return false;
            } // Can only be cast on a weekday
            uint256 hour = _ts / 1 hours % 24;
            if (hour < 14 || hour >= 21) {
                return false;
            } // Outside office hours
        }
        return true;
    }

    /**
     * @dev Calculate the next available cast time in epoch seconds
     * @param _eta          The scheduled time of the spell plus the pause delay
     * @param _ts           The current timestamp, usually block.timestamp
     * @param _officeHours  true if office hours is enabled.
     * @return castTime     The next available cast timestamp
     */
    function nextCastTime(uint40 _eta, uint40 _ts, bool _officeHours) public pure returns (uint256 castTime) {
        require(_eta != 0); // "DssExecLib/invalid eta"
        require(_ts != 0); // "DssExecLib/invalid ts"
        castTime = _ts > _eta ? _ts : _eta; // Any day at XX:YY

        if (_officeHours) {
            uint256 day = (castTime / 1 days + 3) % 7;
            uint256 hour = castTime / 1 hours % 24;
            uint256 minute = castTime / 1 minutes % 60;
            uint256 second = castTime % 60;

            if (day >= 5) {
                castTime += (6 - day) * 1 days; // Go to Sunday XX:YY
                castTime += (24 - hour + 14) * 1 hours; // Go to 14:YY UTC Monday
                castTime -= minute * 1 minutes + second; // Go to 14:00 UTC
            } else {
                if (hour >= 21) {
                    if (day == 4) {
                        castTime += 2 days;
                    } // If Friday, fast forward to Sunday XX:YY
                    castTime += (24 - hour + 14) * 1 hours; // Go to 14:YY UTC next day
                    castTime -= minute * 1 minutes + second; // Go to 14:00 UTC
                } else if (hour < 14) {
                    castTime += (14 - hour) * 1 hours; // Go to 14:YY UTC same day
                    castTime -= minute * 1 minutes + second; // Go to 14:00 UTC
                }
            }
        }
    }

    /**
     *
     */
    /**
     * Accumulating Rates **
     */
    /**
     *
     */
    /**
     * @dev Update rate accumulation for the Dai Savings Rate (DSR).
     */
    function accumulateDSR() public {
        Drippable(pot()).drip();
    }
    /**
     * @dev Update rate accumulation for the stability fees of a given collateral type.
     * @param _ilk   Collateral type
     */

    function accumulateCollateralStabilityFees(bytes32 _ilk) public {
        Drippable(jug()).drip(_ilk);
    }

    /**
     *
     */
    /**
     * Price Updates **
     */
    /**
     *
     */
    /**
     * @dev Update price of a given collateral type.
     * @param _ilk   Collateral type
     */
    function updateCollateralPrice(bytes32 _ilk) public {
        Pricing(spotter()).poke(_ilk);
    }

    /**
     *
     */
    /**
     * System Configuration **
     */
    /**
     *
     */
    /**
     * @dev Set a contract in another contract, defining the relationship (ex. set a new Calc contract in Clip)
     * @param _base   The address of the contract where the new contract address will be filed
     * @param _what   Name of contract to file
     * @param _addr   Address of contract to file
     */
    function setContract(address _base, bytes32 _what, address _addr) public {
        Fileable(_base).file(_what, _addr);
    }
    /**
     * @dev Set a contract in another contract, defining the relationship (ex. set a new Calc contract in a Clip)
     * @param _base   The address of the contract where the new contract address will be filed
     * @param _ilk    Collateral type
     * @param _what   Name of contract to file
     * @param _addr   Address of contract to file
     */

    function setContract(address _base, bytes32 _ilk, bytes32 _what, address _addr) public {
        Fileable(_base).file(_ilk, _what, _addr);
    }
    /**
     * @dev Set a value in a contract, via a governance authorized File pattern.
     * @param _base   The address of the contract where the new contract address will be filed
     * @param _what   Name of tag for the value (e.x. "Line")
     * @param _amt    The value to set or update
     */

    function setValue(address _base, bytes32 _what, uint256 _amt) public {
        Fileable(_base).file(_what, _amt);
    }
    /**
     * @dev Set an ilk-specific value in a contract, via a governance authorized File pattern.
     * @param _base   The address of the contract where the new value will be filed
     * @param _ilk    Collateral type
     * @param _what   Name of tag for the value (e.x. "Line")
     * @param _amt    The value to set or update
     */

    function setValue(address _base, bytes32 _ilk, bytes32 _what, uint256 _amt) public {
        Fileable(_base).file(_ilk, _what, _amt);
    }

    /**
     *
     */
    /**
     * System Risk Parameters **
     */
    /**
     *
     */
    // function setGlobalDebtCeiling(uint256 _amount) public { setGlobalDebtCeiling(vat(), _amount); }
    /**
     * @dev Set the global debt ceiling. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */
    function setGlobalDebtCeiling(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-global-Line-precision"
        setValue(vat(), "Line", _amount * RAD);
    }
    /**
     * @dev Increase the global debt ceiling by a specific amount. Amount will be converted to the correct internal precision.
     * @param _amount The amount to add in DAI (ex. 10m DAI amount == 10000000)
     */

    function increaseGlobalDebtCeiling(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-Line-increase-precision"
        address _vat = vat();
        setValue(_vat, "Line", add(DssVat(_vat).Line(), _amount * RAD));
    }
    /**
     * @dev Decrease the global debt ceiling by a specific amount. Amount will be converted to the correct internal precision.
     * @param _amount The amount to reduce in DAI (ex. 10m DAI amount == 10000000)
     */

    function decreaseGlobalDebtCeiling(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-Line-decrease-precision"
        address _vat = vat();
        setValue(_vat, "Line", sub(DssVat(_vat).Line(), _amount * RAD));
    }
    /**
     * @dev Set the Dai Savings Rate. See: docs/rates.txt
     * @param _rate   The accumulated rate (ex. 4% => 1000000001243680656318820312)
     * @param _doDrip `true` to accumulate interest owed
     */

    function setDSR(uint256 _rate, bool _doDrip) public {
        require((_rate >= RAY) && (_rate <= RATES_ONE_HUNDRED_PCT)); // "LibDssExec/dsr-out-of-bounds"
        if (_doDrip) {
            Drippable(pot()).drip();
        }
        setValue(pot(), "dsr", _rate);
    }
    /**
     * @dev Set the DAI amount for system surplus auctions. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */

    function setSurplusAuctionAmount(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-vow-bump-precision"
        setValue(vow(), "bump", _amount * RAD);
    }
    /**
     * @dev Set the DAI amount for system surplus buffer, must be exceeded before surplus auctions start. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */

    function setSurplusBuffer(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-vow-hump-precision"
        setValue(vow(), "hump", _amount * RAD);
    }
    /**
     * @dev Set minimum bid increase for surplus auctions. Amount will be converted to the correct internal precision.
     * @dev Equation used for conversion is (1 + pct / 10,000) * WAD
     * @param _pct_bps The pct, in basis points, to set in integer form (x100). (ex. 5% = 5 * 100 = 500)
     */

    function setMinSurplusAuctionBidIncrease(uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-flap-beg-precision"
        setValue(flap(), "beg", add(WAD, wdiv(_pct_bps, BPS_ONE_HUNDRED_PCT)));
    }
    /**
     * @dev Set bid duration for surplus auctions.
     * @param _duration Amount of time for bids. (in seconds)
     */

    function setSurplusAuctionBidDuration(uint256 _duration) public {
        setValue(flap(), "ttl", _duration);
    }
    /**
     * @dev Set total auction duration for surplus auctions.
     * @param _duration Amount of time for auctions. (in seconds)
     */

    function setSurplusAuctionDuration(uint256 _duration) public {
        setValue(flap(), "tau", _duration);
    }
    /**
     * @dev Set the number of seconds that pass before system debt is auctioned for MKR tokens.
     * @param _duration Duration in seconds
     */

    function setDebtAuctionDelay(uint256 _duration) public {
        setValue(vow(), "wait", _duration);
    }
    /**
     * @dev Set the DAI amount for system debt to be covered by each debt auction. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */

    function setDebtAuctionDAIAmount(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-vow-sump-precision"
        setValue(vow(), "sump", _amount * RAD);
    }
    /**
     * @dev Set the starting MKR amount to be auctioned off to cover system debt in debt auctions. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in MKR (ex. 250 MKR amount == 250)
     */

    function setDebtAuctionMKRAmount(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-vow-dump-precision"
        setValue(vow(), "dump", _amount * WAD);
    }
    /**
     * @dev Set minimum bid increase for debt auctions. Amount will be converted to the correct internal precision.
     * @dev Equation used for conversion is (1 + pct / 10,000) * WAD
     * @param _pct_bps    The pct, in basis points, to set in integer form (x100). (ex. 5% = 5 * 100 = 500)
     */

    function setMinDebtAuctionBidIncrease(uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-flap-beg-precision"
        setValue(flop(), "beg", add(WAD, wdiv(_pct_bps, BPS_ONE_HUNDRED_PCT)));
    }
    /**
     * @dev Set bid duration for debt auctions.
     * @param _duration Amount of time for bids.
     */

    function setDebtAuctionBidDuration(uint256 _duration) public {
        setValue(flop(), "ttl", _duration);
    }
    /**
     * @dev Set total auction duration for debt auctions.
     * @param _duration Amount of time for auctions.
     */

    function setDebtAuctionDuration(uint256 _duration) public {
        setValue(flop(), "tau", _duration);
    }
    /**
     * @dev Set the rate of increasing amount of MKR out for auction during debt auctions. Amount will be converted to the correct internal precision.
     * @dev MKR amount is increased by this rate every "tick" (if auction duration has passed and no one has bid on the MKR)
     * @dev Equation used for conversion is (1 + pct / 10,000) * WAD
     * @param _pct_bps    The pct, in basis points, to set in integer form (x100). (ex. 5% = 5 * 100 = 500)
     */

    function setDebtAuctionMKRIncreaseRate(uint256 _pct_bps) public {
        setValue(flop(), "pad", add(WAD, wdiv(_pct_bps, BPS_ONE_HUNDRED_PCT)));
    }
    /**
     * @dev Set the maximum total DAI amount that can be out for liquidation in the system at any point. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in DAI (ex. 250,000 DAI amount == 250000)
     */

    function setMaxTotalDAILiquidationAmount(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-dog-Hole-precision"
        setValue(dog(), "Hole", _amount * RAD);
    }
    /**
     * @dev (LIQ 1.2) Set the maximum total DAI amount that can be out for liquidation in the system at any point. Amount will be converted to the correct internal precision.
     * @param _amount The amount to set in DAI (ex. 250,000 DAI amount == 250000)
     */

    function setMaxTotalDAILiquidationAmountLEGACY(uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-cat-box-amount"
        setValue(cat(), "box", _amount * RAD);
    }
    /**
     * @dev Set the duration of time that has to pass during emergency shutdown before collateral can start being claimed by DAI holders.
     * @param _duration Time in seconds to set for ES processing time
     */

    function setEmergencyShutdownProcessingTime(uint256 _duration) public {
        setValue(end(), "wait", _duration);
    }
    /**
     * @dev Set the global stability fee (is not typically used, currently is 0).
     * Many of the settings that change weekly rely on the rate accumulator
     * described at https://docs.makerdao.com/smart-contract-modules/rates-module
     * To check this yourself, use the following rate calculation (example 8%):
     *
     * $ bc -l <<< 'scale=27; e( l(1.08)/(60 * 60 * 24 * 365) )'
     *
     * A table of rates can also be found at:
     * https://ipfs.io/ipfs/QmefQMseb3AiTapiAKKexdKHig8wroKuZbmLtPLv4u2YwW
     * @param _rate   The accumulated rate (ex. 4% => 1000000001243680656318820312)
     */

    function setGlobalStabilityFee(uint256 _rate) public {
        require((_rate >= RAY) && (_rate <= RATES_ONE_HUNDRED_PCT)); // "LibDssExec/global-stability-fee-out-of-bounds"
        setValue(jug(), "base", _rate);
    }
    /**
     * @dev Set the value of DAI in the reference asset (e.g. $1 per DAI). Value will be converted to the correct internal precision.
     * @dev Equation used for conversion is value * RAY / 1000
     * @param _value The value to set as integer (x1000) (ex. $1.025 == 1025)
     */

    function setDAIReferenceValue(uint256 _value) public {
        require(_value < WAD); // "LibDssExec/incorrect-par-precision"
        setValue(spotter(), "par", rdiv(_value, 1000));
    }

    /**
     *
     */
    /**
     * Collateral Management **
     */
    /**
     *
     */
    /**
     * @dev Set a collateral debt ceiling. Amount will be converted to the correct internal precision.
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */
    function setIlkDebtCeiling(bytes32 _ilk, uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-ilk-line-precision"
        setValue(vat(), _ilk, "line", _amount * RAD);
    }
    /**
     * @dev Increase a collateral debt ceiling. Amount will be converted to the correct internal precision.
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The amount to increase in DAI (ex. 10m DAI amount == 10000000)
     * @param _global If true, increases the global debt ceiling by _amount
     */

    function increaseIlkDebtCeiling(bytes32 _ilk, uint256 _amount, bool _global) public {
        require(_amount < WAD); // "LibDssExec/incorrect-ilk-line-precision"
        address _vat = vat();
        (,,, uint256 line_,) = DssVat(_vat).ilks(_ilk);
        setValue(_vat, _ilk, "line", add(line_, _amount * RAD));
        if (_global) {
            increaseGlobalDebtCeiling(_amount);
        }
    }
    /**
     * @dev Decrease a collateral debt ceiling. Amount will be converted to the correct internal precision.
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The amount to decrease in DAI (ex. 10m DAI amount == 10000000)
     * @param _global If true, decreases the global debt ceiling by _amount
     */

    function decreaseIlkDebtCeiling(bytes32 _ilk, uint256 _amount, bool _global) public {
        require(_amount < WAD); // "LibDssExec/incorrect-ilk-line-precision"
        address _vat = vat();
        (,,, uint256 line_,) = DssVat(_vat).ilks(_ilk);
        setValue(_vat, _ilk, "line", sub(line_, _amount * RAD));
        if (_global) {
            decreaseGlobalDebtCeiling(_amount);
        }
    }
    /**
     * @dev Set the parameters for an ilk in the "MCD_IAM_AUTO_LINE" auto-line
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The Maximum value (ex. 100m DAI amount == 100000000)
     * @param _gap    The amount of Dai per step (ex. 5m Dai == 5000000)
     * @param _ttl    The amount of time (in seconds)
     */

    function setIlkAutoLineParameters(bytes32 _ilk, uint256 _amount, uint256 _gap, uint256 _ttl) public {
        require(_amount < WAD); // "LibDssExec/incorrect-auto-line-amount-precision"
        require(_gap < WAD); // "LibDssExec/incorrect-auto-line-gap-precision"
        IAMLike(autoLine()).setIlk(_ilk, _amount * RAD, _gap * RAD, _ttl);
    }
    /**
     * @dev Set the debt ceiling for an ilk in the "MCD_IAM_AUTO_LINE" auto-line without updating the time values
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The Maximum value (ex. 100m DAI amount == 100000000)
     */

    function setIlkAutoLineDebtCeiling(bytes32 _ilk, uint256 _amount) public {
        address _autoLine = autoLine();
        (, uint256 gap, uint48 ttl,,) = IAMLike(_autoLine).ilks(_ilk);
        require(gap != 0 && ttl != 0); // "LibDssExec/auto-line-not-configured"
        IAMLike(_autoLine).setIlk(_ilk, _amount * RAD, uint256(gap), uint256(ttl));
    }
    /**
     * @dev Remove an ilk in the "MCD_IAM_AUTO_LINE" auto-line
     * @param _ilk    The ilk to remove (ex. bytes32("ETH-A"))
     */

    function removeIlkFromAutoLine(bytes32 _ilk) public {
        IAMLike(autoLine()).remIlk(_ilk);
    }
    /**
     * @dev Set a collateral minimum vault amount. Amount will be converted to the correct internal precision.
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */

    function setIlkMinVaultAmount(bytes32 _ilk, uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-ilk-dust-precision"
        setValue(vat(), _ilk, "dust", _amount * RAD);
        (bool ok,) = clip(_ilk).call(abi.encodeWithSignature("upchost()"));
        ok;
    }
    /**
     * @dev Set a collateral liquidation penalty. Amount will be converted to the correct internal precision.
     * @dev Equation used for conversion is (1 + pct / 10,000) * WAD
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _pct_bps    The pct, in basis points, to set in integer form (x100). (ex. 10.25% = 10.25 * 100 = 1025)
     */

    function setIlkLiquidationPenalty(bytes32 _ilk, uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-ilk-chop-precision"
        setValue(dog(), _ilk, "chop", add(WAD, wdiv(_pct_bps, BPS_ONE_HUNDRED_PCT)));
        (bool ok,) = clip(_ilk).call(abi.encodeWithSignature("upchost()"));
        ok;
    }
    /**
     * @dev Set max DAI amount for liquidation per vault for collateral. Amount will be converted to the correct internal precision.
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The amount to set in DAI (ex. 10m DAI amount == 10000000)
     */

    function setIlkMaxLiquidationAmount(bytes32 _ilk, uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-ilk-hole-precision"
        setValue(dog(), _ilk, "hole", _amount * RAD);
    }
    /**
     * @dev Set a collateral liquidation ratio. Amount will be converted to the correct internal precision.
     * @dev Equation used for conversion is pct * RAY / 10,000
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _pct_bps    The pct, in basis points, to set in integer form (x100). (ex. 150% = 150 * 100 = 15000)
     */

    function setIlkLiquidationRatio(bytes32 _ilk, uint256 _pct_bps) public {
        require(_pct_bps < 10 * BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-ilk-mat-precision" // Fails if pct >= 1000%
        require(_pct_bps >= BPS_ONE_HUNDRED_PCT); // the liquidation ratio has to be bigger or equal to 100%
        setValue(spotter(), _ilk, "mat", rdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
    }
    /**
     * @dev Set an auction starting multiplier. Amount will be converted to the correct internal precision.
     * @dev Equation used for conversion is pct * RAY / 10,000
     * @param _ilk      The ilk to update (ex. bytes32("ETH-A"))
     * @param _pct_bps  The pct, in basis points, to set in integer form (x100). (ex. 1.3x starting multiplier = 130% = 13000)
     */

    function setStartingPriceMultiplicativeFactor(bytes32 _ilk, uint256 _pct_bps) public {
        require(_pct_bps < 10 * BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-ilk-mat-precision" // Fails if gt 10x
        require(_pct_bps >= BPS_ONE_HUNDRED_PCT); // fail if start price is less than OSM price
        setValue(clip(_ilk), "buf", rdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
    }

    /**
     * @dev Set the amount of time before an auction resets.
     * @param _ilk      The ilk to update (ex. bytes32("ETH-A"))
     * @param _duration Amount of time before auction resets (in seconds).
     */
    function setAuctionTimeBeforeReset(bytes32 _ilk, uint256 _duration) public {
        setValue(clip(_ilk), "tail", _duration);
    }

    /**
     * @dev Percentage drop permitted before auction reset
     * @param _ilk     The ilk to update (ex. bytes32("ETH-A"))
     * @param _pct_bps The pct, in basis points, of drop to permit (x100).
     */
    function setAuctionPermittedDrop(bytes32 _ilk, uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-clip-cusp-value"
        setValue(clip(_ilk), "cusp", rdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
    }

    /**
     * @dev Percentage of tab to suck from vow to incentivize keepers. Amount will be converted to the correct internal precision.
     * @param _ilk     The ilk to update (ex. bytes32("ETH-A"))
     * @param _pct_bps The pct, in basis points, of the tab to suck. (0.01% == 1)
     */
    function setKeeperIncentivePercent(bytes32 _ilk, uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-clip-chip-precision"
        setValue(clip(_ilk), "chip", wdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
    }

    /**
     * @dev Set max DAI amount for flat rate keeper incentive. Amount will be converted to the correct internal precision.
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A"))
     * @param _amount The amount to set in DAI (ex. 1000 DAI amount == 1000)
     */
    function setKeeperIncentiveFlatRate(bytes32 _ilk, uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-clip-tip-precision"
        setValue(clip(_ilk), "tip", _amount * RAD);
    }

    /**
     * @dev Sets the circuit breaker price tolerance in the clipper mom.
     * This is somewhat counter-intuitive,
     * to accept a 25% price drop, use a value of 75%
     * @param _clip    The clipper to set the tolerance for
     * @param _pct_bps The pct, in basis points, to set in integer form (x100). (ex. 5% = 5 * 100 = 500)
     */
    function setLiquidationBreakerPriceTolerance(address _clip, uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // "LibDssExec/incorrect-clippermom-price-tolerance"
        MomLike(clipperMom()).setPriceTolerance(_clip, rdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
    }

    /**
     * @dev Set the stability fee for a given ilk.
     * Many of the settings that change weekly rely on the rate accumulator
     * described at https://docs.makerdao.com/smart-contract-modules/rates-module
     * To check this yourself, use the following rate calculation (example 8%):
     *
     * $ bc -l <<< 'scale=27; e( l(1.08)/(60 * 60 * 24 * 365) )'
     *
     * A table of rates can also be found at:
     * https://ipfs.io/ipfs/QmefQMseb3AiTapiAKKexdKHig8wroKuZbmLtPLv4u2YwW
     *
     * @param _ilk    The ilk to update (ex. bytes32("ETH-A") )
     * @param _rate   The accumulated rate (ex. 4% => 1000000001243680656318820312)
     * @param _doDrip `true` to accumulate stability fees for the collateral
     */
    function setIlkStabilityFee(bytes32 _ilk, uint256 _rate, bool _doDrip) public {
        require((_rate >= RAY) && (_rate <= RATES_ONE_HUNDRED_PCT)); // "LibDssExec/ilk-stability-fee-out-of-bounds"
        address _jug = jug();
        if (_doDrip) {
            Drippable(_jug).drip(_ilk);
        }

        setValue(_jug, _ilk, "duty", _rate);
    }

    /**
     *
     */
    /**
     * Abacus Management **
     */
    /**
     *
     */

    /**
     * @dev Set the number of seconds from the start when the auction reaches zero price.
     * @dev Abacus:LinearDecrease only.
     * @param _calc     The address of the LinearDecrease pricing contract
     * @param _duration Amount of time for auctions.
     */
    function setLinearDecrease(address _calc, uint256 _duration) public {
        setValue(_calc, "tau", _duration);
    }

    /**
     * @dev Set the number of seconds for each price step.
     * @dev Abacus:StairstepExponentialDecrease only.
     * @param _calc     The address of the StairstepExponentialDecrease pricing contract
     * @param _duration Length of time between price drops [seconds]
     * @param _pct_bps Per-step multiplicative factor in basis points. (ex. 99% == 9900)
     */
    function setStairstepExponentialDecrease(address _calc, uint256 _duration, uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // DssExecLib/cut-too-high
        setValue(_calc, "cut", rdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
        setValue(_calc, "step", _duration);
    }
    /**
     * @dev Set the number of seconds for each price step. (99% cut = 1% price drop per step)
     * Amounts will be converted to the correct internal precision.
     * @dev Abacus:ExponentialDecrease only
     * @param _calc     The address of the ExponentialDecrease pricing contract
     * @param _pct_bps Per-step multiplicative factor in basis points. (ex. 99% == 9900)
     */

    function setExponentialDecrease(address _calc, uint256 _pct_bps) public {
        require(_pct_bps < BPS_ONE_HUNDRED_PCT); // DssExecLib/cut-too-high
        setValue(_calc, "cut", rdiv(_pct_bps, BPS_ONE_HUNDRED_PCT));
    }

    /**
     *
     */
    /**
     * Oracle Management **
     */
    /**
     *
     */
    /**
     * @dev Allows an oracle to read prices from its source feeds
     * @param _oracle  An OSM or LP oracle contract
     */
    function whitelistOracleMedians(address _oracle) public {
        (bool ok, bytes memory data) = _oracle.call(abi.encodeWithSignature("orb0()"));
        if (ok) {
            // Token is an LP oracle
            address median0 = abi.decode(data, (address));
            addReaderToWhitelistCall(median0, _oracle);
            addReaderToWhitelistCall(OracleLike(_oracle).orb1(), _oracle);
        } else {
            // Standard OSM
            addReaderToWhitelistCall(OracleLike(_oracle).src(), _oracle);
        }
    }
    /**
     * @dev Adds an address to the OSM or Median's reader whitelist, allowing the address to read prices.
     * @param _oracle        Oracle Security Module (OSM) or Median core contract address
     * @param _reader     Address to add to whitelist
     */

    function addReaderToWhitelist(address _oracle, address _reader) public {
        OracleLike(_oracle).kiss(_reader);
    }
    /**
     * @dev Removes an address to the OSM or Median's reader whitelist, disallowing the address to read prices.
     * @param _oracle     Oracle Security Module (OSM) or Median core contract address
     * @param _reader     Address to remove from whitelist
     */

    function removeReaderFromWhitelist(address _oracle, address _reader) public {
        OracleLike(_oracle).diss(_reader);
    }
    /**
     * @dev Adds an address to the OSM or Median's reader whitelist, allowing the address to read prices.
     * @param _oracle  OSM or Median core contract address
     * @param _reader  Address to add to whitelist
     */

    function addReaderToWhitelistCall(address _oracle, address _reader) public {
        (bool ok,) = _oracle.call(abi.encodeWithSignature("kiss(address)", _reader));
        ok;
    }
    /**
     * @dev Removes an address to the OSM or Median's reader whitelist, disallowing the address to read prices.
     * @param _oracle  Oracle Security Module (OSM) or Median core contract address
     * @param _reader  Address to remove from whitelist
     */

    function removeReaderFromWhitelistCall(address _oracle, address _reader) public {
        (bool ok,) = _oracle.call(abi.encodeWithSignature("diss(address)", _reader));
        ok;
    }
    /**
     * @dev Sets the minimum number of valid messages from whitelisted oracle feeds needed to update median price.
     * @param _median     Median core contract address
     * @param _minQuorum  Minimum number of valid messages from whitelisted oracle feeds needed to update median price (NOTE: MUST BE ODD NUMBER)
     */

    function setMedianWritersQuorum(address _median, uint256 _minQuorum) public {
        OracleLike(_median).setBar(_minQuorum);
    }
    /**
     * @dev Add OSM address to OSM mom, allowing it to be frozen by governance.
     * @param _osm        Oracle Security Module (OSM) core contract address
     * @param _ilk        Collateral type using OSM
     */

    function allowOSMFreeze(address _osm, bytes32 _ilk) public {
        MomLike(osmMom()).setOsm(_ilk, _osm);
    }

    /**
     *
     */
    /**
     * Collateral Onboarding **
     */
    /**
     *
     */

    /**
     * @dev Performs basic functions and sanity checks to add a new collateral type to the MCD system
     * @param _ilk      Collateral type key code [Ex. "ETH-A"]
     * @param _gem      Address of token contract
     * @param _join     Address of join adapter
     * @param _clip     Address of liquidation agent
     * @param _calc     Address of the pricing function
     * @param _pip      Address of price feed
     */
    function addCollateralBase(bytes32 _ilk, address _gem, address _join, address _clip, address _calc, address _pip)
        public
    {
        // Sanity checks
        address _vat = vat();
        address _dog = dog();
        address _spotter = spotter();
        require(JoinLike(_join).vat() == _vat); // "join-vat-not-match"
        require(JoinLike(_join).ilk() == _ilk); // "join-ilk-not-match"
        require(JoinLike(_join).gem() == _gem); // "join-gem-not-match"
        require(JoinLike(_join).dec() == ERC20(_gem).decimals()); // "join-dec-not-match"
        require(ClipLike(_clip).vat() == _vat); // "clip-vat-not-match"
        require(ClipLike(_clip).dog() == _dog); // "clip-dog-not-match"
        require(ClipLike(_clip).ilk() == _ilk); // "clip-ilk-not-match"
        require(ClipLike(_clip).spotter() == _spotter); // "clip-ilk-not-match"

        // Set the token PIP in the Spotter
        setContract(spotter(), _ilk, "pip", _pip);

        // Set the ilk Clipper in the Dog
        setContract(_dog, _ilk, "clip", _clip);
        // Set vow in the clip
        setContract(_clip, "vow", vow());
        // Set the pricing function for the Clipper
        setContract(_clip, "calc", _calc);

        // Init ilk in Vat & Jug
        Initializable(_vat).init(_ilk); // Vat
        Initializable(jug()).init(_ilk); // Jug

        // Allow ilk Join to modify Vat registry
        authorize(_vat, _join);
        // Allow ilk Join to suck dai for keepers
        authorize(_vat, _clip);
        // Allow the ilk Clipper to reduce the Dog hole on deal()
        authorize(_dog, _clip);
        // Allow Dog to kick auctions in ilk Clipper
        authorize(_clip, _dog);
        // Allow End to yank auctions in ilk Clipper
        authorize(_clip, end());
        // Authorize the ESM to execute in the clipper
        authorize(_clip, esm());

        // Add new ilk to the IlkRegistry
        RegistryLike(reg()).add(_join);
    }

    // Complete collateral onboarding logic.
    function addNewCollateral(CollateralOpts memory co) public {
        // Add the collateral to the system.
        addCollateralBase(co.ilk, co.gem, co.join, co.clip, co.calc, co.pip);
        address clipperMom_ = clipperMom();

        if (!co.isLiquidatable) {
            // Disallow Dog to kick auctions in ilk Clipper
            setValue(co.clip, "stopped", 3);
        } else {
            // Grant ClipperMom access to the ilk Clipper
            authorize(co.clip, clipperMom_);
        }

        if (co.isOSM) {
            // If pip == OSM
            // Allow OsmMom to access to the TOKEN OSM
            authorize(co.pip, osmMom());
            if (co.whitelistOSM) {
                // If median is src in OSM
                // Whitelist OSM to read the Median data (only necessary if it is the first time the token is being added to an ilk)
                whitelistOracleMedians(co.pip);
            }
            // Whitelist Spotter to read the OSM data (only necessary if it is the first time the token is being added to an ilk)
            addReaderToWhitelist(co.pip, spotter());
            // Whitelist Clipper on pip
            addReaderToWhitelist(co.pip, co.clip);
            // Allow the clippermom to access the feed
            addReaderToWhitelist(co.pip, clipperMom_);
            // Whitelist End to read the OSM data (only necessary if it is the first time the token is being added to an ilk)
            addReaderToWhitelist(co.pip, end());
            // Set TOKEN OSM in the OsmMom for new ilk
            allowOSMFreeze(co.pip, co.ilk);
        }
        // Increase the global debt ceiling by the ilk ceiling
        increaseGlobalDebtCeiling(co.ilkDebtCeiling);
        // Set the ilk debt ceiling
        setIlkDebtCeiling(co.ilk, co.ilkDebtCeiling);
        // Set the ilk dust
        setIlkMinVaultAmount(co.ilk, co.minVaultAmount);
        // Set the hole size
        setIlkMaxLiquidationAmount(co.ilk, co.maxLiquidationAmount);
        // Set the ilk liquidation penalty
        setIlkLiquidationPenalty(co.ilk, co.liquidationPenalty);

        // Set the ilk stability fee
        setIlkStabilityFee(co.ilk, co.ilkStabilityFee, true);

        // Set the auction starting price multiplier
        setStartingPriceMultiplicativeFactor(co.ilk, co.startingPriceFactor);

        // Set the amount of time before an auction resets.
        setAuctionTimeBeforeReset(co.ilk, co.auctionDuration);

        // Set the allowed auction drop percentage before reset
        setAuctionPermittedDrop(co.ilk, co.permittedDrop);

        // Set the ilk min collateralization ratio
        setIlkLiquidationRatio(co.ilk, co.liquidationRatio);

        // Set the price tolerance in the liquidation circuit breaker
        setLiquidationBreakerPriceTolerance(co.clip, co.breakerTolerance);

        // Set a flat rate for the keeper reward
        setKeeperIncentiveFlatRate(co.ilk, co.kprFlatReward);

        // Set the percentage of liquidation as keeper award
        setKeeperIncentivePercent(co.ilk, co.kprPctReward);

        // Update ilk spot value in Vat
        updateCollateralPrice(co.ilk);
    }

    /**
     *
     */
    /**
     * Payment **
     */
    /**
     *
     */
    /**
     * @dev Send a payment in ERC20 DAI from the surplus buffer.
     * @param _target The target address to send the DAI to.
     * @param _amount The amount to send in DAI (ex. 10m DAI amount == 10000000)
     */
    function sendPaymentFromSurplusBuffer(address _target, uint256 _amount) public {
        require(_amount < WAD); // "LibDssExec/incorrect-ilk-line-precision"
        DssVat(vat()).suck(vow(), address(this), _amount * RAD);
        JoinLike(daiJoin()).exit(_target, _amount * WAD);
    }

    /**
     *
     */
    /**
     * Misc **
     */
    /**
     *
     */
    /**
     * @dev Initiate linear interpolation on an administrative value over time.
     * @param _name        The label for this lerp instance
     * @param _target      The target contract
     * @param _what        The target parameter to adjust
     * @param _startTime   The time for this lerp
     * @param _start       The start value for the target parameter
     * @param _end         The end value for the target parameter
     * @param _duration    The duration of the interpolation
     */
    function linearInterpolation(
        bytes32 _name,
        address _target,
        bytes32 _what,
        uint256 _startTime,
        uint256 _start,
        uint256 _end,
        uint256 _duration
    ) public returns (address) {
        address lerp = LerpFactoryLike(lerpFab()).newLerp(_name, _target, _what, _startTime, _start, _end, _duration);
        Authorizable(_target).rely(lerp);
        LerpLike(lerp).tick();
        return lerp;
    }
    /**
     * @dev Initiate linear interpolation on an administrative value over time.
     * @param _name        The label for this lerp instance
     * @param _target      The target contract
     * @param _ilk         The ilk to target
     * @param _what        The target parameter to adjust
     * @param _startTime   The time for this lerp
     * @param _start       The start value for the target parameter
     * @param _end         The end value for the target parameter
     * @param _duration    The duration of the interpolation
     */

    function linearInterpolation(
        bytes32 _name,
        address _target,
        bytes32 _ilk,
        bytes32 _what,
        uint256 _startTime,
        uint256 _start,
        uint256 _end,
        uint256 _duration
    ) public returns (address) {
        address lerp =
            LerpFactoryLike(lerpFab()).newIlkLerp(_name, _target, _ilk, _what, _startTime, _start, _end, _duration);
        Authorizable(_target).rely(lerp);
        LerpLike(lerp).tick();
        return lerp;
    }
}
