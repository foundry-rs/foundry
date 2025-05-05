// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Vm} from "forge-std/Vm.sol";

contract VyperDeployer {
    address private constant HEVM_ADDRESS = address(uint160(uint256(keccak256("hevm cheat code"))));
    Vm private constant vm = Vm(HEVM_ADDRESS);
    // Base directory for Vyper contracts
    string private constant BASE_PATH = "src/";

    /**
     * Compiles a Vyper contract and returns the `CREATE` address.
     * @param fileName The file name of the Vyper contract.
     * @return deployedAddress The address calculated through create operation.
     */
    function deployContract(string memory fileName) public returns (address) {
        return deployContract(BASE_PATH, fileName, "");
    }

    /**
     * Compiles a Vyper contract and returns the `CREATE` address.
     * @param fileName The file name of the Vyper contract.
     * @param args The constructor arguments for the contract
     * @return deployedAddress The address calculated through create operation.
     */
    function deployContract(string memory fileName, bytes memory args) public returns (address) {
        return deployContract(BASE_PATH, fileName, args);
    }

    /**
     * Compiles a Vyper contract with constructor arguments and returns the `CREATE` address.
     * @param basePath The base directory path where the Vyper contract is located
     * @param fileName The file name of the Vyper contract.
     * @param args The constructor arguments for the contract
     * @return deployedAddress The address calculated through create operation.
     */
    function deployContract(string memory basePath, string memory fileName, bytes memory args)
        public
        returns (address)
    {
        // Compile the Vyper contract
        bytes memory bytecode = compileVyperContract(basePath, fileName);

        // Add constructor arguments if provided
        if (args.length > 0) {
            bytecode = abi.encodePacked(bytecode, args);
        }

        // Deploy the contract
        address deployedAddress = deployBytecode(bytecode);

        // Return the deployed address
        return deployedAddress;
    }

    /**
     * Compiles a Vyper contract and returns the bytecode
     * @param basePath The base directory path where the Vyper contract is located
     * @param fileName The file name of the Vyper contract
     * @return The compiled bytecode of the contract
     */
    function compileVyperContract(string memory basePath, string memory fileName) internal returns (bytes memory) {
        // create a list of strings with the commands necessary to compile Vyper contracts
        string[] memory cmds = new string[](2);
        cmds[0] = "vyper";
        cmds[1] = string.concat(basePath, fileName, ".vy");

        // compile the Vyper contract and return the bytecode
        return vm.ffi(cmds);
    }

    /**
     * Deploys bytecode using the create instruction
     * @param bytecode - The bytecode to deploy
     * @return deployedAddress The address calculated through create operation.
     */
    function deployBytecode(bytes memory bytecode) internal returns (address deployedAddress) {
        // deploy the bytecode with the create instruction
        assembly {
            deployedAddress := create(0, add(bytecode, 0x20), mload(bytecode))
        }

        // check that the deployment was successful
        require(deployedAddress != address(0), "VyperDeployer could not deploy contract");
    }
}
