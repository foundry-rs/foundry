interface IActivationRegistry {
    function admin() external view returns (address);
    function isActivated(bytes32 feature) external view returns (bool);
    function activate(bytes32 feature) external;
}

interface Vm {
    function prank(address sender) external;
    function deal(address account, uint256 newBalance) external;
}

contract BaseForkWritesTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant ACTIVATION_REGISTRY = 0x8453000000000000000000000000000000000001;
    bytes32 constant B20_ASSET = 0xcdcc772fe4cbdb1029f822861176d09e646db96723d4c1e82ddfdeb8163ef54c;

    // `activate` returns nothing, so Solidity guards the high-level call with an `extcodesize`
    // check. That check is why a code-less precompile makes the caller revert before the
    // precompile runs, so this must stay a high-level call rather than a raw `.call()`.
    function test_fork_activation_write() public {
        address admin = IActivationRegistry(ACTIVATION_REGISTRY).admin();
        require(admin != address(0), "activation admin unresolved");
        require(!IActivationRegistry(ACTIVATION_REGISTRY).isActivated(B20_ASSET), "already active");

        vm.deal(admin, 1 ether);
        vm.prank(admin);
        IActivationRegistry(ACTIVATION_REGISTRY).activate(B20_ASSET);

        require(IActivationRegistry(ACTIVATION_REGISTRY).isActivated(B20_ASSET), "not activated");
    }
}
