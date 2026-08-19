interface IActivationRegistry {
    function admin() external view returns (address);
    function isActivated(bytes32 feature) external view returns (bool);
    function activate(bytes32 feature) external;
}

interface IB20Factory {
    function getB20Address(uint8 variant, address sender, bytes32 salt) external view returns (address);
    function isB20(address token) external view returns (bool);
}

interface Vm {
    function prank(address sender) external;
    function deal(address account, uint256 newBalance) external;
}

contract BaseForkWritesTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant ACTIVATION_REGISTRY = 0x8453000000000000000000000000000000000001;
    address constant B20_FACTORY = 0xB20f000000000000000000000000000000000000;
    bytes32 constant B20_ASSET = 0xcdcc772fe4cbdb1029f822861176d09e646db96723d4c1e82ddfdeb8163ef54c;

    // The factory returns data from every function, so Solidity emits no `extcodesize` check
    // against it and it needs no code — which is why Base leaves it code-less on chain. Planting a
    // sentinel here anyway would let an `isContract` probe pass locally and revert on Base.
    function test_factory_is_code_less_and_still_callable() public view {
        require(B20_FACTORY.code.length == 0, "factory must stay code-less, as on Base");

        address token =
            IB20Factory(B20_FACTORY).getB20Address(0, address(this), bytes32(uint256(1)));
        require(token != address(0), "high-level factory call should return an address");
        require(!IB20Factory(B20_FACTORY).isB20(address(this)), "test contract is not a B20");
    }

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
