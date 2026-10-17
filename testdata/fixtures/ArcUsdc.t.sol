// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface Vm {
    function createFork(string calldata url, uint256 blockNumber) external returns (uint256);
    function createSelectFork(string calldata url, uint256 blockNumber) external returns (uint256);
    function selectFork(uint256 forkId) external;
    function deal(address account, uint256 balance) external;
    function etch(address target, bytes calldata code) external;
    function prank(address sender) external;
    function store(address target, bytes32 slot, bytes32 value) external;
}

interface IArcUsdc {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
}

contract RevertingArcUsdcTransfer {
    function transferAndRevert(IArcUsdc usdc, address recipient) external {
        require(usdc.transfer(recipient, 3_000_000));
        revert("rollback transfer");
    }
}

contract ArcUsdcTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    IArcUsdc constant usdc = IArcUsdc(0x3600000000000000000000000000000000000000);
    address constant AUTHORITY = 0x1800000000000000000000000000000000000000;
    address constant CONTROL = 0x1800000000000000000000000000000000000001;
    address constant SENDER = address(0xA11CE);
    address constant RECIPIENT = address(0xB0B);

    function setUp() public virtual {
        vm.deal(SENDER, 10 ether);
        vm.deal(RECIPIENT, 0);
    }

    function testForkUsdcTransfer() public {
        require(block.chainid == 5_042_002, "wrong fork chain");
        require(usdc.balanceOf(SENDER) == 10_000_000, "initial ERC20 balance");
        vm.prank(SENDER);
        require(usdc.transfer(RECIPIENT, 3_000_000), "transfer returned false");
        require(usdc.balanceOf(SENDER) == 7_000_000, "sender ERC20 balance");
        require(usdc.balanceOf(RECIPIENT) == 3_000_000, "recipient ERC20 balance");
        require(SENDER.balance == 7 ether, "sender native balance");
        require(RECIPIENT.balance == 3 ether, "recipient native balance");
    }

    function testForkUsdcTransferFrom() public {
        vm.prank(SENDER);
        require(usdc.approve(address(this), 4_000_000), "approve returned false");
        require(usdc.transferFrom(SENDER, RECIPIENT, 3_000_000), "transferFrom returned false");
        require(usdc.allowance(SENDER, address(this)) == 1_000_000, "allowance");
        require(SENDER.balance == 7 ether, "sender native balance");
        require(RECIPIENT.balance == 3 ether, "recipient native balance");
    }

    function testForkUsdcTransferRollsBack() public {
        RevertingArcUsdcTransfer helper = new RevertingArcUsdcTransfer();
        vm.deal(address(helper), 10 ether);
        (bool success, bytes memory result) =
            address(helper).call(abi.encodeCall(helper.transferAndRevert, (usdc, RECIPIENT)));
        require(!success, "expected outer revert");
        require(
            keccak256(result) == keccak256(abi.encodeWithSignature("Error(string)", "rollback transfer")),
            "wrong revert"
        );
        require(address(helper).balance == 10 ether, "sender transfer not reverted");
        require(RECIPIENT.balance == 0, "recipient transfer not reverted");
    }

    function testForkUsdcAuthorityRejectsUnauthorizedCaller() public {
        (bool success, bytes memory result) =
            AUTHORITY.call(abi.encodeWithSignature("transfer(address,address,uint256)", SENDER, RECIPIENT, 3 ether));
        require(!success, "unauthorized native transfer succeeded");
        require(
            keccak256(result)
                == keccak256(abi.encodeWithSignature("Error(string)", "Not enabled for native coin transfers")),
            "wrong revert"
        );
        require(SENDER.balance == 10 ether, "sender changed");
        require(RECIPIENT.balance == 0, "recipient changed");
    }

    function testForkUsdcZeroAndSelfTransfer() public {
        vm.prank(SENDER);
        require(usdc.transfer(RECIPIENT, 0), "zero transfer returned false");
        vm.prank(SENDER);
        require(usdc.transfer(SENDER, 3_000_000), "self transfer returned false");
        require(SENDER.balance == 10 ether, "sender changed");
        require(RECIPIENT.balance == 0, "recipient changed");
    }

    function testForkUsdcTransferFromRejectsBlockedSpender() public {
        vm.prank(SENDER);
        require(usdc.approve(address(this), 4_000_000), "approve returned false");
        vm.store(CONTROL, keccak256(abi.encode(address(this), uint256(2))), bytes32(uint256(1)));
        (bool success,) = address(usdc).call(abi.encodeCall(usdc.transferFrom, (SENDER, RECIPIENT, 3_000_000)));
        require(!success, "blocklisted spender transferred funds");
        require(SENDER.balance == 10 ether, "sender changed");
        require(RECIPIENT.balance == 0, "recipient changed");
    }

    function testForkUsdcTransferRejectsBlockedRecipient() public {
        vm.store(CONTROL, keccak256(abi.encode(RECIPIENT, uint256(2))), bytes32(uint256(1)));
        vm.prank(SENDER);
        (bool success,) = address(usdc).call(abi.encodeCall(usdc.transfer, (RECIPIENT, 3_000_000)));
        require(!success, "blocklisted transfer succeeded");
        require(SENDER.balance == 10 ether, "sender changed");
        require(RECIPIENT.balance == 0, "recipient changed");
    }
}
