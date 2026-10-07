// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ExecuteTransactionTracingTest is Test {
    uint256 internal calls;

    function test_nested_leaf() public {
        this.execute(signedTransaction(abi.encodeCall(this.leaf, ()), false, false));
    }

    function test_nested_subcall() public {
        this.execute(signedTransaction(abi.encodeCall(this.subcall, ()), false, false));
        this.leaf();
        assertEq(calls, 3);
    }

    function test_nested_create() public {
        this.execute(signedTransaction(type(TraceChild).creationCode, true, false));
    }

    function test_nested_transaction_creates_child() public {
        this.execute(signedTransaction(abi.encodeCall(this.createChild, ()), false, false));
    }

    function test_recursive_transaction() public {
        this.execute(signedTransaction(abi.encodeCall(this.recursive, ()), false, false));
    }

    function test_nested_revert() public {
        this.executeFailure(
            signedTransaction(abi.encodeCall(this.revertTransaction, ()), false, false),
            string.concat(
                "vm.executeTransaction: transaction reverted: ",
                vm.toString(abi.encodeWithSignature("Error(string)", "nested failure"))
            )
        );
        assertEq(calls, 1);
    }

    function test_nested_invalid_nonce() public {
        this.executeFailure(
            signedTransaction(abi.encodeCall(this.leaf, ()), false, true),
            "vm.executeTransaction: transaction execution failed: transaction validation error: nonce 1 too high, expected 0"
        );
    }

    function test_nested_halt() public {
        this.executeFailure(
            signedTransaction(abi.encodeCall(this.haltTransaction, ()), false, false),
            "vm.executeTransaction: transaction halted: InvalidFEOpcode"
        );
    }

    function execute(bytes memory rawTx) external {
        address origin = tx.origin;
        vm.executeTransaction(rawTx);
        assertEq(tx.origin, origin);
        this.leaf();
    }

    function executeFailure(bytes memory rawTx, string memory expectedError) external {
        address origin = tx.origin;
        try vm.executeTransaction(rawTx) {
            revert("expected transaction failure");
        } catch (bytes memory reason) {
            assertEq(reason, abi.encodeWithSignature("CheatcodeError(string)", expectedError));
        }
        assertEq(tx.origin, origin);
        this.leaf();
    }

    function recursive() external {
        this.execute(signedTransaction(abi.encodeCall(this.subcall, ()), false, false));
    }

    function subcall() external {
        require(tx.origin == 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf, "nested origin");
        this.leaf();
    }

    function createChild() external {
        new TraceChild();
    }

    function leaf() external {
        calls++;
    }

    function revertTransaction() external {
        this.leaf();
        revert("nested failure");
    }

    function haltTransaction() external pure {
        assembly {
            invalid()
        }
    }

    function signedTransaction(bytes memory data, bool create, bool invalidNonce) internal returns (bytes memory) {
        uint256 privateKey = 1;
        vm.chainId(1);
        vm.deal(vm.addr(privateKey), 1 ether);

        bytes[] memory unsigned = new bytes[](9);
        unsigned[0] = trimLeadingZeros(bytes32(uint256(vm.getNonce(vm.addr(privateKey))) + (invalidNonce ? 1 : 0)));
        unsigned[1] = hex"01";
        unsigned[2] = hex"030d40";
        unsigned[3] = create ? bytes("") : abi.encodePacked(address(this));
        unsigned[5] = data;
        unsigned[6] = hex"01";

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, keccak256(vm.toRlp(unsigned)));
        unsigned[6] = abi.encodePacked(v + 10);
        unsigned[7] = trimLeadingZeros(r);
        unsigned[8] = trimLeadingZeros(s);
        return vm.toRlp(unsigned);
    }

    function trimLeadingZeros(bytes32 value_) internal pure returns (bytes memory out) {
        uint256 offset;
        while (offset < 32 && value_[offset] == bytes1(0)) {
            offset++;
        }
        out = new bytes(32 - offset);
        for (uint256 i; i < out.length; i++) {
            out[i] = value_[offset + i];
        }
    }
}

contract TraceChild {}
