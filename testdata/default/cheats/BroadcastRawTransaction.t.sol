// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract BroadcastRawTransactionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_revert_not_a_tx() public {
        vm._expectCheatcodeRevert("failed to decode RLP-encoded transaction: unexpected string");
        vm.broadcastRawTransaction(hex"0102");
    }

    function test_revert_missing_signature() public {
        vm._expectCheatcodeRevert("failed to decode RLP-encoded transaction: input too short");
        vm.broadcastRawTransaction(hex"dd806483030d40940993863c19b0defb183ca2b502db7d1b331ded757b80");
    }

    function test_revert_wrong_chainid() public {
        vm._expectCheatcodeRevert("transaction validation error: invalid chain ID");
        vm.broadcastRawTransaction(
            hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856"
        );
    }

    function test_execute_signed_tx() public {
        vm.fee(1);
        vm.chainId(1);

        address from = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
        address to = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;

        uint256 balance = 1 ether;
        uint256 amountSent = 17;

        vm.deal(address(from), balance);
        assertEq(address(from).balance, balance);
        assertEq(address(to).balance, 0);

        /*
        Signed transaction:
        TransactionRequest { from: Some(0x5316812db67073c4d4af8bb3000c5b86c2877e94), to: Some(Address(0x6fd0a0cff9a87adf51695b40b4fa267855a8f4c6)), gas: Some(200000), gas_price: Some(100), value: Some(17), data: None, nonce: Some(0), chain_id: Some(1) }
        */
        vm.broadcastRawTransaction(
            hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856"
        );

        uint256 gasPrice = 100;
        assertEq(address(from).balance, balance - (gasPrice * 21_000) - amountSent);
        assertEq(address(to).balance, amountSent);
    }

    function test_execute_signed_tx2() public {
        vm.fee(1);
        vm.chainId(1);

        address from = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
        address to = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
        address random = address(uint160(uint256(keccak256(abi.encodePacked("random")))));

        uint256 balance = 1 ether;
        uint256 amountSent = 17;

        vm.deal(address(from), balance);
        assertEq(address(from).balance, balance);
        assertEq(address(to).balance, 0);

        /*
        Signed transaction:
        TransactionRequest { from: Some(0x5316812db67073c4d4af8bb3000c5b86c2877e94), to: Some(Address(0x6fd0a0cff9a87adf51695b40b4fa267855a8f4c6)), gas: Some(200000), gas_price: Some(100), value: Some(17), data: None, nonce: Some(0), chain_id: Some(1) }
        */
        vm.broadcastRawTransaction(
            hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856"
        );

        uint256 gasPrice = 100;
        assertEq(address(from).balance, balance - (gasPrice * 21_000) - amountSent);
        assertEq(address(to).balance, amountSent);
        assertEq(address(random).balance, 0);

        uint256 value = 5;

        vm.prank(to);
        (bool success,) = random.call{value: value}("");
        require(success);
        assertEq(address(to).balance, amountSent - value);
        assertEq(address(random).balance, value);
    }

    // this test is to make sure that the journaledstate is correctly handled
    // i ran into an issue where the test would fail after running `broadcastRawTransaction`
    // because there was an issue in the journaledstate
    function test_execute_signed_tx_with_revert() public {
        vm.fee(1);
        vm.chainId(1);

        address from = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
        address to = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;

        uint256 balance = 1 ether;
        uint256 amountSent = 17;

        vm.deal(address(from), balance);
        assertEq(address(from).balance, balance);
        assertEq(address(to).balance, 0);

        /*
        Signed transaction:
        TransactionRequest { from: Some(0x5316812db67073c4d4af8bb3000c5b86c2877e94), to: Some(Address(0x6fd0a0cff9a87adf51695b40b4fa267855a8f4c6)), gas: Some(200000), gas_price: Some(100), value: Some(17), data: None, nonce: Some(0), chain_id: Some(1) }
        */
        vm.broadcastRawTransaction(
            hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856"
        );

        uint256 gasPrice = 100;
        assertEq(address(from).balance, balance - (gasPrice * 21_000) - amountSent);
        assertEq(address(to).balance, amountSent);

        vm.expectRevert();
        assert(3 == 4);
    }

    function test_execute_multiple_signed_tx() public {
        vm.fee(1);
        vm.chainId(1);

        address alice = 0x7ED31830602f9F7419307235c0610Fb262AA0375;
        address bob = 0x70CF146aB98ffD5dE24e75dd7423F16181Da8E13;
        address charlie = 0xae0900Cf97f8C233c64F7089cEC7d5457215BB8d;

        // this is the runtime code of "MyERC20" (see below)
        // this is equivalent to:
        // type(MyERC20).runtimeCode
        bytes memory code =
            hex"608060405234801561001057600080fd5b50600436106100625760003560e01c8063095ea7b31461006757806323b872dd1461008f57806370a08231146100a257806394bf804d146100d9578063a9059cbb146100ee578063dd62ed3e14610101575b600080fd5b61007a61007536600461051d565b61013a565b60405190151581526020015b60405180910390f35b61007a61009d366004610547565b610152565b6100cb6100b0366004610583565b6001600160a01b031660009081526020819052604090205490565b604051908152602001610086565b6100ec6100e73660046105a5565b610176565b005b61007a6100fc36600461051d565b610184565b6100cb61010f3660046105d1565b6001600160a01b03918216600090815260016020908152604080832093909416825291909152205490565b600033610148818585610192565b5060019392505050565b600033610160858285610286565b61016b858585610318565b506001949350505050565b6101808183610489565b5050565b600033610148818585610318565b6001600160a01b0383166101f95760405162461bcd60e51b8152602060048201526024808201527f45524332303a20617070726f76652066726f6d20746865207a65726f206164646044820152637265737360e01b60648201526084015b60405180910390fd5b6001600160a01b03821661025a5760405162461bcd60e51b815260206004820152602260248201527f45524332303a20617070726f766520746f20746865207a65726f206164647265604482015261737360f01b60648201526084016101f0565b6001600160a01b0392831660009081526001602090815260408083209490951682529290925291902055565b6001600160a01b03838116600090815260016020908152604080832093861683529290522054600019811461031257818110156103055760405162461bcd60e51b815260206004820152601d60248201527f45524332303a20696e73756666696369656e7420616c6c6f77616e636500000060448201526064016101f0565b6103128484848403610192565b50505050565b6001600160a01b03831661037c5760405162461bcd60e51b815260206004820152602560248201527f45524332303a207472616e736665722066726f6d20746865207a65726f206164604482015264647265737360d81b60648201526084016101f0565b6001600160a01b0382166103de5760405162461bcd60e51b815260206004820152602360248201527f45524332303a207472616e7366657220746f20746865207a65726f206164647260448201526265737360e81b60648201526084016101f0565b6001600160a01b038316600090815260208190526040902054818110156104565760405162461bcd60e51b815260206004820152602660248201527f45524332303a207472616e7366657220616d6f756e7420657863656564732062604482015265616c616e636560d01b60648201526084016101f0565b6001600160a01b039384166000908152602081905260408082209284900390925592909316825291902080549091019055565b6001600160a01b0382166104df5760405162461bcd60e51b815260206004820152601f60248201527f45524332303a206d696e7420746f20746865207a65726f20616464726573730060448201526064016101f0565b6001600160a01b03909116600090815260208190526040902080549091019055565b80356001600160a01b038116811461051857600080fd5b919050565b6000806040838503121561053057600080fd5b61053983610501565b946020939093013593505050565b60008060006060848603121561055c57600080fd5b61056584610501565b925061057360208501610501565b9150604084013590509250925092565b60006020828403121561059557600080fd5b61059e82610501565b9392505050565b600080604083850312156105b857600080fd5b823591506105c860208401610501565b90509250929050565b600080604083850312156105e457600080fd5b6105ed83610501565b91506105c86020840161050156fea2646970667358221220e1fee5cd1c5bbf066a9ce9228e1baf7e7fcb77b5050506c7d614aaf8608b42e364736f6c63430008110033";

        // this is equivalent to:
        // MyERC20 token = new MyERC20{ salt: bytes32(uint256(1)) }();
        // address: 0x5bf11839f61ef5cceeaf1f4153e44df5d02825f7
        MyERC20 token = MyERC20(address(uint160(uint256(keccak256(abi.encodePacked("mytoken"))))));
        vm.etch(address(token), code);

        token.mint(100, alice);

        assertEq(token.balanceOf(alice), 100);
        assertEq(token.balanceOf(bob), 0);
        assertEq(token.balanceOf(charlie), 0);

        vm.deal(alice, 10 ether);

        /*
        Signed transaction:
        {
            from: '0x7ED31830602f9F7419307235c0610Fb262AA0375',
            to: '0x5bF11839F61EF5ccEEaf1F4153e44df5D02825f7',
            value: 0,
            data: '0x095ea7b300000000000000000000000070cf146ab98ffd5de24e75dd7423f16181da8e130000000000000000000000000000000000000000000000000000000000000032',
            nonce: 0,
            gasPrice: 100,
            gasLimit: 200000,
            chainId: 1
        }
        */
        // this would be equivalent to using those cheatcodes:
        // vm.prank(alice);
        // token.approve(bob, 50);
        vm.broadcastRawTransaction(
            hex"f8a5806483030d40945bf11839f61ef5cceeaf1f4153e44df5d02825f780b844095ea7b300000000000000000000000070cf146ab98ffd5de24e75dd7423f16181da8e13000000000000000000000000000000000000000000000000000000000000003225a0e25b9ef561d9a413b21755cc0e4bb6e80f2a88a8a52305690956130d612074dfa07bfd418bc2ad3c3f435fa531cdcdc64887f64ed3fb0d347d6b0086e320ad4eb1"
        );

        assertEq(token.allowance(alice, bob), 50);

        vm.deal(bob, 1 ether);
        vm.prank(bob);
        token.transferFrom(alice, charlie, 20);

        assertEq(token.balanceOf(bob), 0);
        assertEq(token.balanceOf(charlie), 20);

        vm.deal(charlie, 1 ether);

        /*
        Signed transaction:
        {
            from: '0xae0900Cf97f8C233c64F7089cEC7d5457215BB8d',
            to: '0x5bF11839F61EF5ccEEaf1F4153e44df5D02825f7',
            value: 0,
            data: '0xa9059cbb00000000000000000000000070cf146ab98ffd5de24e75dd7423f16181da8e130000000000000000000000000000000000000000000000000000000000000005',
            nonce: 0,
            gasPrice: 100,
            gasLimit: 200000,
            chainId: 1
        }
        */
        // this would be equivalent to using those cheatcodes:
        // vm.prank(charlie);
        // token.transfer(bob, 5);
        vm.broadcastRawTransaction(
            hex"f8a5806483030d40945bf11839f61ef5cceeaf1f4153e44df5d02825f780b844a9059cbb00000000000000000000000070cf146ab98ffd5de24e75dd7423f16181da8e13000000000000000000000000000000000000000000000000000000000000000525a0941562f519e33dfe5b44ebc2b799686cebeaeacd617dd89e393620b380797da2a0447dfd38d9444ccd571b000482c81674733761753430c81ee6669e9542c266a1"
        );

        assertEq(token.balanceOf(alice), 80);
        assertEq(token.balanceOf(bob), 5);
        assertEq(token.balanceOf(charlie), 15);
    }
}

contract MyERC20 {
    mapping(address => uint256) private _balances;
    mapping(address => mapping(address => uint256)) private _allowances;

    function mint(uint256 amount, address to) public {
        _mint(to, amount);
    }

    function balanceOf(address account) public view returns (uint256) {
        return _balances[account];
    }

    function transfer(address to, uint256 amount) public returns (bool) {
        address owner = msg.sender;
        _transfer(owner, to, amount);
        return true;
    }

    function allowance(address owner, address spender) public view returns (uint256) {
        return _allowances[owner][spender];
    }

    function approve(address spender, uint256 amount) public returns (bool) {
        address owner = msg.sender;
        _approve(owner, spender, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) public returns (bool) {
        address spender = msg.sender;
        _spendAllowance(from, spender, amount);
        _transfer(from, to, amount);
        return true;
    }

    function _transfer(address from, address to, uint256 amount) internal {
        require(from != address(0), "ERC20: transfer from the zero address");
        require(to != address(0), "ERC20: transfer to the zero address");

        uint256 fromBalance = _balances[from];
        require(fromBalance >= amount, "ERC20: transfer amount exceeds balance");
        unchecked {
            _balances[from] = fromBalance - amount;
            _balances[to] += amount;
        }
    }

    function _mint(address account, uint256 amount) internal {
        require(account != address(0), "ERC20: mint to the zero address");
        unchecked {
            _balances[account] += amount;
        }
    }

    function _approve(address owner, address spender, uint256 amount) internal {
        require(owner != address(0), "ERC20: approve from the zero address");
        require(spender != address(0), "ERC20: approve to the zero address");
        _allowances[owner][spender] = amount;
    }

    function _spendAllowance(address owner, address spender, uint256 amount) internal {
        uint256 currentAllowance = allowance(owner, spender);
        if (currentAllowance != type(uint256).max) {
            require(currentAllowance >= amount, "ERC20: insufficient allowance");
            unchecked {
                _approve(owner, spender, currentAllowance - amount);
            }
        }
    }
}

contract ScriptBroadcastRawTransactionBroadcast is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function runSignedTxBroadcast() public {
        uint256 pk_to = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
        vm.startBroadcast(pk_to);

        address from = 0x73E1A965542AFA4B412467761b1CED8A764E1D3B;
        address to = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
        address random = address(uint160(uint256(keccak256(abi.encodePacked("random")))));

        assert(address(from).balance == 1 ether);
        assert(address(to).balance == 1 ether);
        assert(address(random).balance == 0);

        /*
            TransactionRequest {
                from: Some(
                    0x73e1a965542afa4b412467761b1ced8a764e1d3b,
                ),
                to: Some(
                    Address(
                        0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266,
                    ),
                ),
                gas: Some(
                    200000,
                ),
                gas_price: Some(
                    10000000000,
                ),
                value: Some(
                    1234,
                ),
                data: None,
                nonce: Some(
                    0,
                ),
                chain_id: Some(
                    31337,
                ),
            }
        */
        vm.broadcastRawTransaction(
            hex"f869808502540be40083030d4094f39fd6e51aad88f6f4ce6ab8827279cfffb922668204d28082f4f6a061ce3c0f4280cb79c1eb0060a9a491cca1ba48ed32f141e3421ccb60c9dbe444a07fcd35cbec5f81427ac20f60484f4da9d00f59652f5053cd13ee90b992e94ab3"
        );

        uint256 value = 34;
        (bool success,) = random.call{value: value}("");
        require(success);

        vm.stopBroadcast();

        uint256 gasPrice = 10 * 1e9;
        assert(address(from).balance == 1 ether - (gasPrice * 21_000) - 1234);
        assert(address(to).balance == 1 ether + 1234 - value);
        assert(address(random).balance == value);
    }

    function runDeployCreate2Deployer() public {
        vm.startBroadcast();
        vm.broadcastRawTransaction(
            hex"f8a58085174876e800830186a08080b853604580600e600039806000f350fe7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf31ba02222222222222222222222222222222222222222222222222222222222222222a02222222222222222222222222222222222222222222222222222222222222222"
        );
        vm.stopBroadcast();
    }
}
