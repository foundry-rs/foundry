// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IGasPriceOracle {
    function overhead() external view returns (uint256);
    function scalar() external view returns (uint256);
    function getL1GasUsed(bytes memory data) external view returns (uint256);
    function baseFee() external view returns (uint256);
}

contract OptimismDeprecation {
    // SHOULD FAIL, deprecated predeploy address literals

    function useLegacyMessagePasser() public {
        address target = 0x4200000000000000000000000000000000000000; //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
        (bool ok,) = target.call("");
        require(ok);
    }

    function useL1MessageSender() public view returns (address) {
        return address(0x4200000000000000000000000000000000000001); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    function useDeployerWhitelist() public {
        IGasPriceOracle(0x4200000000000000000000000000000000000002).baseFee(); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    function useL1BlockNumber() public view returns (uint256) {
        return IGasPriceOracle(0x4200000000000000000000000000000000000013).baseFee(); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    // SHOULD FAIL, deprecated GasPriceOracle functions (revert post-Ecotone)

    function getOverhead() public view returns (uint256) {
        return IGasPriceOracle(0x420000000000000000000000000000000000000F).overhead(); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    function getScalar() public view returns (uint256) {
        return IGasPriceOracle(0x420000000000000000000000000000000000000F).scalar(); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    function getL1GasUsed(bytes memory data) public view returns (uint256) {
        return IGasPriceOracle(0x420000000000000000000000000000000000000F).getL1GasUsed(data); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    // SHOULD PASS, GPO address used, but only calling a non-deprecated function

    function getBaseFee() public view returns (uint256) {
        return IGasPriceOracle(0x420000000000000000000000000000000000000F).baseFee();
    }

    // SHOULD FAIL, GPO accessed via aliased local variable

    function gpoViaAlias() public view returns (uint256) {
        IGasPriceOracle gpo = IGasPriceOracle(0x420000000000000000000000000000000000000F);
        return gpo.overhead(); //~WARN: usage of a deprecated Optimism predeploy address or GasPriceOracle function that reverts post-Ecotone
    }

    // SHOULD PASS, GPO accessed via state variable constant as state vars are not tracked

    IGasPriceOracle constant GPO = IGasPriceOracle(0x420000000000000000000000000000000000000F);

    function gpoViaStateConst() public view returns (uint256) {
        return GPO.overhead();
    }

    // SHOULD PASS, unrelated hex literals

    function unrelatedHex() public pure returns (uint256) {
        uint256 mask = 0xffffffff;
        return mask;
    }

    function unrelatedAddress() public pure returns (address) {
        return address(0x1234567890123456789012345678901234567890);
    }
}
