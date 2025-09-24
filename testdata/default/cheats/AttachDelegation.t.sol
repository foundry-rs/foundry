// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract AttachDelegationTest is DSTest {
    event ExecutedBy(uint256 id);

    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 alice_pk = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    address payable alice = payable(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
    uint256 bob_pk = 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;
    address bob = 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC;

    SimpleDelegateContract implementation;
    SimpleDelegateContract implementation2;
    ERC20 token;

    function setUp() public {
        implementation = new SimpleDelegateContract(1);
        implementation2 = new SimpleDelegateContract(2);
        token = new ERC20(alice);
    }

    function testCallSingleAttachDelegation() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk);
        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({to: address(token), data: data, value: 0});
        // executing as bob to make clear that we don't need to execute the tx as alice
        vm.broadcast(bob_pk);
        vm.attachDelegation(signedDelegation);

        bytes memory code = address(alice).code;
        require(code.length > 0, "no code written to alice");
        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 100);
    }

    function testCallSingleAttachCrossChainDelegation() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk, true);
        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({to: address(token), data: data, value: 0});
        // executing as bob to make clear that we don't need to execute the tx as alice
        vm.broadcast(bob_pk);
        vm.attachDelegation(signedDelegation, true);

        bytes memory code = address(alice).code;
        require(code.length > 0, "no code written to alice");
        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 100);
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testCallSingleAttachDelegationWithNonce() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk, 11);
        vm.broadcast(bob_pk);
        vm._expectCheatcodeRevert("vm.attachDelegation: invalid nonce");
        vm.attachDelegation(signedDelegation);

        signedDelegation = vm.signDelegation(address(implementation), alice_pk, 0);
        vm.attachDelegation(signedDelegation);
    }

    function testMultiCallAttachDelegation() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk);
        vm.broadcast(bob_pk);
        vm.attachDelegation(signedDelegation);

        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](2);
        calls[0] =
            SimpleDelegateContract.Call({to: address(token), data: abi.encodeCall(ERC20.mint, (50, bob)), value: 0});
        calls[1] = SimpleDelegateContract.Call({
            to: address(token), data: abi.encodeCall(ERC20.mint, (50, address(this))), value: 0
        });

        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 50);
        assertEq(token.balanceOf(address(this)), 50);
    }

    function testMultiCallAttachCrossChainDelegation() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk, true);
        vm.broadcast(bob_pk);
        vm.attachDelegation(signedDelegation, true);

        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](2);
        calls[0] =
            SimpleDelegateContract.Call({to: address(token), data: abi.encodeCall(ERC20.mint, (50, bob)), value: 0});
        calls[1] = SimpleDelegateContract.Call({
            to: address(token), data: abi.encodeCall(ERC20.mint, (50, address(this))), value: 0
        });

        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 50);
        assertEq(token.balanceOf(address(this)), 50);
    }

    function testSwitchAttachDelegation() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk);

        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({to: address(token), data: data, value: 0});

        vm.broadcast(bob_pk);
        vm.attachDelegation(signedDelegation);

        vm.expectEmit(true, true, true, true);
        emit ExecutedBy(1);
        SimpleDelegateContract(alice).execute(calls);

        // switch to implementation2
        Vm.SignedDelegation memory signedDelegation2 = vm.signDelegation(address(implementation2), alice_pk);
        vm.broadcast(bob_pk);
        vm.attachDelegation(signedDelegation2);

        vm.expectEmit(true, true, true, true);
        emit ExecutedBy(2);
        SimpleDelegateContract(alice).execute(calls);

        // verify final state
        assertEq(token.balanceOf(bob), 200);
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testAttachDelegationRevertInvalidSignature() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk);
        // change v from 1 to 0
        signedDelegation.v = (signedDelegation.v + 1) % 2;
        vm.attachDelegation(signedDelegation);

        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({to: address(token), data: data, value: 0});

        vm.broadcast(alice_pk);
        // empty revert because no bytecode was set to Alice's account
        vm.expectRevert();
        SimpleDelegateContract(alice).execute(calls);
    }

    function testAttachDelegationRevertsAfterNonceChange() public {
        Vm.SignedDelegation memory signedDelegation = vm.signDelegation(address(implementation), alice_pk);

        vm.broadcast(alice_pk);
        // send tx to increment alice's nonce
        token.mint(1, bob);

        vm._expectCheatcodeRevert("vm.attachDelegation: invalid nonce");
        vm.attachDelegation(signedDelegation);
    }

    function testCallSingleSignAndAttachDelegation() public {
        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({to: address(token), data: data, value: 0});
        vm.signAndAttachDelegation(address(implementation), alice_pk);
        bytes memory code = address(alice).code;
        require(code.length > 0, "no code written to alice");
        vm.broadcast(bob_pk);
        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 100);
    }

    function testCallSingleSignAndAttachCrossChainDelegation() public {
        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({to: address(token), data: data, value: 0});
        vm.signAndAttachDelegation(address(implementation), alice_pk, true);
        bytes memory code = address(alice).code;
        require(code.length > 0, "no code written to alice");
        vm.broadcast(bob_pk);
        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 100);
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testCallSingleSignAndAttachDelegationWithNonce() public {
        vm._expectCheatcodeRevert("vm.signAndAttachDelegation: invalid nonce");
        vm.signAndAttachDelegation(address(implementation), alice_pk, 11);

        vm.signAndAttachDelegation(address(implementation), alice_pk, 0);
    }

    function testMultipleDelegationsOnTransaction() public {
        vm.signAndAttachDelegation(address(implementation), alice_pk);
        vm.signAndAttachDelegation(address(implementation2), bob_pk);
        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](2);
        calls[0] = SimpleDelegateContract.Call({
            to: address(token), data: abi.encodeCall(ERC20.mint, (50, address(this))), value: 0
        });
        calls[1] = SimpleDelegateContract.Call({
            to: address(token), data: abi.encodeCall(ERC20.mint, (50, alice)), value: 0
        });
        vm.broadcast(bob_pk);
        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(address(this)), 50);
        assertEq(token.balanceOf(alice), 50);

        vm._expectCheatcodeRevert("vm.signAndAttachDelegation: invalid nonce");
        vm.signAndAttachDelegation(address(implementation), alice_pk, 1);
        vm.signAndAttachDelegation(address(implementation), alice_pk, 0);
        vm.signAndAttachDelegation(address(implementation2), bob_pk, 2);
    }
}

contract SimpleDelegateContract {
    event Executed(address indexed to, uint256 value, bytes data);
    event ExecutedBy(uint256 id);

    struct Call {
        bytes data;
        address to;
        uint256 value;
    }

    uint256 public immutable id;

    constructor(uint256 _id) {
        id = _id;
    }

    function execute(Call[] memory calls) external payable {
        for (uint256 i = 0; i < calls.length; i++) {
            Call memory call = calls[i];
            (bool success, bytes memory result) = call.to.call{value: call.value}(call.data);
            require(success, string(result));
            emit Executed(call.to, call.value, call.data);
            emit ExecutedBy(id);
        }
    }

    receive() external payable {}
}

contract ERC20 {
    address public minter;
    mapping(address => uint256) private _balances;

    constructor(address _minter) {
        minter = _minter;
    }

    function mint(uint256 amount, address to) public {
        _mint(to, amount);
    }

    function balanceOf(address account) public view returns (uint256) {
        return _balances[account];
    }

    function _mint(address account, uint256 amount) internal {
        require(msg.sender == minter, "ERC20: msg.sender is not minter");
        require(account != address(0), "ERC20: mint to the zero address");
        unchecked {
            _balances[account] += amount;
        }
    }
}
