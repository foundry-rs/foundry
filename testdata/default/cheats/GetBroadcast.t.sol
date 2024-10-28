// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetBroadcastTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_getLatestBroadcast() public {
        // Get latest Counter deployment on chain_id 31337
        bytes32 latestCreate = 0xf4320023569ddcc3aab7da8a39b7388a3aeadf32da466a4b10709274999dcff5;

        Vm.BroadcastTxSummary memory broadcast = vm.getBroadcast("Counter", 31337, Vm.BroadcastTxType.Create);

        assertEq(broadcast.txHash, latestCreate);

        bytes32 latestCreate2 = 0x008bbaec35131ffc7de43d362aa4412e6970bcd8c48d531e9fda51ceaf1f5fe9;

        Vm.BroadcastTxSummary memory broadcast2 = vm.getBroadcast("Counter", 31337, Vm.BroadcastTxType.Create2);

        assertEq(broadcast2.txHash, latestCreate2);

        bytes32 latestCall = 0x1046225989e019746b245e8b746933795371efe39c946f3b18bc43d70e260d9f;

        Vm.BroadcastTxSummary memory broadcast3 = vm.getBroadcast("Counter", 1, Vm.BroadcastTxType.Call);

        assertEq(broadcast3.txHash, latestCall);
    }

    function test_getBroadcasts() public {
        // Gets the calls to Counter on chain_id 31337
        Vm.BroadcastTxSummary[] memory broadcasts = vm.getBroadcasts("Counter", 31337, Vm.BroadcastTxType.Call);

        assertEq(broadcasts.length, 2);

        bytes32 call1 = 0xfc4831d57d8f9eddb686a6c6e10a468df70a5838700319b2780e9a7c6fdbbbcf;
        assertEq(broadcasts[0].txHash, call1);

        bytes32 call2 = 0xf65e225fd1cfdb621876d4d8faa8391647c7117931a57d2ffeeafc7e3f0eb3b7;
        assertEq(broadcasts[1].txHash, call2);
    }

    function test_getAllBroadcasts() public {
        // Gets all broadcasts to Counter on chain_id 31337
        Vm.BroadcastTxSummary[] memory broadcasts = vm.getBroadcasts("Counter", 31337);

        assertEq(broadcasts.length, 4);

        uint8 num_calls = 0;
        uint8 num_create = 0;
        uint8 num_create2 = 0;

        for (uint256 i = 0; i < broadcasts.length; i++) {
            if (broadcasts[i].txType == Vm.BroadcastTxType.Call) {
                num_calls++;
            } else if (broadcasts[i].txType == Vm.BroadcastTxType.Create) {
                num_create++;
            } else if (broadcasts[i].txType == Vm.BroadcastTxType.Create2) {
                num_create2++;
            }
        }

        assertEq(num_calls, 2);
        assertEq(num_create, 1);
        assertEq(num_create2, 1);
    }

    function test_getDeployment() public {
        // Get the most recent deployment of Counter on chain_id 31337
        address deployment = vm.getDeployment("Counter", 31337);

        assertEq(deployment, address(0x22c6191707bB9C1de43Fd7e72De3775e2f018516));
    }

    function test_getDeployments() public {
        // Get all deployments of Counter on chain_id 31337
        address[] memory deployments = vm.getDeployments("Counter", 31337);

        assertEq(deployments.length, 2);

        assertEq(deployments[0], address(0x22c6191707bB9C1de43Fd7e72De3775e2f018516));

        assertEq(deployments[1], address(0x5FbDB2315678afecb367f032d93F642f64180aa3));
    }
}
