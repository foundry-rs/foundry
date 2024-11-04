// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract AttachDelegationTest is DSTest {
    event Executed(address indexed to, uint256 value, bytes data);
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 alice_pk = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    address payable alice = payable(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
    uint256 bob_pk=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;
    address bob = 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC;

    function testAttachDelegation() public {
        SimpleDelegateContract implementation = new SimpleDelegateContract();
        ERC20 token = new ERC20(alice);

        (uint8 v, bytes32 r, bytes32 s) = vm.signDelegation(address(implementation), alice_pk);
        SimpleDelegateContract.Call[] memory calls = new SimpleDelegateContract.Call[](1);
        bytes memory data = abi.encodeCall(ERC20.mint, (100, bob));
        calls[0] = SimpleDelegateContract.Call({
            to: address(token),
            data: data,
            value: 0
        });
        // executing as bob to make clear that we don't need to execute the tx as alice
        vm.broadcast(bob_pk);
        vm.attachDelegation(address(implementation), alice, v, r, s);

        bytes memory code = address(alice).code;
        require(code.length > 0, "no code written to alice");
        SimpleDelegateContract(alice).execute(calls);

        assertEq(token.balanceOf(bob), 100);
    }
}

contract SimpleDelegateContract {
    event WhoAmI(address);
    event Executed(address indexed to, uint256 value, bytes data);
    struct Call {
        bytes data;
        address to;
        uint256 value;
    }

    function execute(Call[] memory calls) external payable {
        emit WhoAmI(msg.sender);
        for (uint256 i = 0; i < calls.length; i++) {
            emit WhoAmI(address(this));
            Call memory call = calls[i];
            (bool success, bytes memory result) = call.to.call{value: call.value}(call.data);
            require(success, string(result));
            emit Executed(call.to, call.value, call.data);
        }
    }

    receive() external payable {}
}

contract ERC20 {
    event WhoAmI(address);
    address public minter;
    mapping(address => uint256) private _balances;

    constructor(address _minter) {
        minter = _minter;
        emit WhoAmI(minter);
    }

    function mint(uint256 amount, address to) public {
        _mint(to, amount);
    }

    function balanceOf(address account) public view returns (uint256) {
        return _balances[account];
    }

    function _mint(address account, uint256 amount) internal {
        emit WhoAmI(msg.sender);
        emit WhoAmI(tx.origin);
        require(msg.sender == minter, "ERC20: msg.sender is not minter");
        require(account != address(0), "ERC20: mint to the zero address");
        unchecked {
            _balances[account] += amount;
        }
    }
}