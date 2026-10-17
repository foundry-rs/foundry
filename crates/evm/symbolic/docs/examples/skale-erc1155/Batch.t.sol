// SPDX-License-Identifier: AGPL-3.0-only
pragma solidity 0.8.6;

import {
    TokenManagerERC1155,
    IMessageProxyForSchain,
    ITokenManagerLinker,
    ICommunityLocker,
    ERC1155OnChain,
    Messages
} from "../src/schain/TokenManagers/TokenManagerERC1155.sol";

interface Vm {
    function startSnapshotGas(string calldata name) external;
    function stopSnapshotGas() external returns (uint256);
    function assume(bool condition) external;
}

contract BatchTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 constant COUNT = 39;
    address constant ORIGINAL = address(0x1234);
    address constant DEPOSIT_BOX = address(0x5678);
    address constant RECEIVER = address(0x9abc);
    TokenManagerERC1155 manager;
    ERC1155OnChain token;

    function setUp() public {
        manager = new TokenManagerERC1155();
        manager.initialize(
            "Demo",
            IMessageProxyForSchain(address(this)),
            ITokenManagerLinker(address(0)),
            ICommunityLocker(address(0)),
            DEPOSIT_BOX
        );
        token = new ERC1155OnChain("");
        token.grantRole(token.MINTER_ROLE(), address(manager));
        manager.addERC1155TokenByOwner("Mainnet", ORIGINAL, address(token));
    }

    // Registration calls this stub. The measured message does not.
    function isConnectedChain(string calldata) external pure returns (bool) {
        return true;
    }

    function payload(uint8[COUNT] memory quantities) internal pure returns (bytes memory) {
        uint256[] memory ids = new uint256[](COUNT);
        uint256[] memory amounts = new uint256[](COUNT);
        for (uint256 i; i < COUNT; i++) {
            ids[i] = i + 1;
            amounts[i] = quantities[i];
        }
        return abi.encodeWithSelector(
            TokenManagerERC1155.postMessage.selector,
            keccak256("Mainnet"),
            DEPOSIT_BOX,
            Messages.encodeTransferErc1155BatchMessage(ORIGINAL, RECEIVER, ids, amounts)
        );
    }

    function checkBudget(uint8[COUNT] memory quantities) public {
        for (uint256 i; i < COUNT; i++) {
            vm.assume(quantities[i] > 0);
        }
        bytes memory data = payload(quantities);
        vm.startSnapshotGas("batch");
        (bool ok,) = address(manager).call(data);
        uint256 used = vm.stopSnapshotGas();
        require(ok, "mint failed");
        assert(used <= 1_000_000);
    }
}
