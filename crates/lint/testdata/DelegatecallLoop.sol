//@compile-flags: --severity low

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract DelegatecallLoop {
    function payableForLoop(bytes[] calldata payloads) external payable {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = target.delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
        }
    }

    function payableWhileLoop(bytes[] calldata payloads) external payable {
        address target = address(this);
        uint256 i;
        while (i < payloads.length) {
            (bool ok,) = target.delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
            ++i;
        }
    }

    function payableDoWhileLoop(bytes[] calldata payloads) external payable {
        address target = address(this);
        uint256 i;
        if (payloads.length == 0) return;
        do {
            (bool ok,) = target.delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
            ++i;
        } while (i < payloads.length);
    }

    function payableNestedDelegatecall(bytes[] calldata payloads) external payable {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            if (payloads[i].length != 0) {
                (bool ok,) = target.delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
                require(ok);
            }
        }
    }

    function payableForUpdateExpression(bytes[] calldata payloads) external payable {
        address target = address(this);
        bool ok = true;
        for (
            uint256 i = 0;
            i < payloads.length;
            (ok,) = target.delegatecall(payloads[i++]) //~WARN: payable functions should not use `delegatecall` inside a loop
        ) {}
        require(ok);
    }

    modifier loopDelegatecall(bytes[] calldata payloads) {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = target.delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
        }
        _;
    }

    function payableModifierLoop(bytes[] calldata payloads) external payable loopDelegatecall(payloads) {}

    function payableLoopWithCallAndStaticcall(bytes[] calldata payloads) external payable {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool callOk,) = target.call(payloads[i]);
            (bool staticcallOk,) = target.staticcall(payloads[i]);
            require(callOk && staticcallOk);
        }
    }

    function payableDelegatecallOutsideLoop(bytes calldata payload) external payable {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload);
        require(ok);
    }

    function nonPayableLoop(bytes[] calldata payloads) external {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = target.delegatecall(payloads[i]);
            require(ok);
        }
    }
}
