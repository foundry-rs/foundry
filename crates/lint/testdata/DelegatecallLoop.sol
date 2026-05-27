//@compile-flags: --only-lint delegatecall-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

library DelegatecallLoopLib {
    function helper(bytes calldata payload) internal {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload); //~WARN: payable functions should not use `delegatecall` inside a loop
        require(ok);
    }
}

interface OrdinaryDelegatecall {
    function delegatecall(bytes calldata payload) external returns (bool);
}

contract ParentOrdinaryDelegatecall {
    function delegatecall(bytes calldata) public pure virtual returns (bool) {
        return true;
    }
}

contract ParentDelegatecallHelper {
    function superDelegate(bytes calldata payload) internal {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload); //~WARN: payable functions should not use `delegatecall` inside a loop
        require(ok);
    }
}

contract LinearizedSuperBase {
    function next(bytes calldata) internal virtual {}
}

contract LinearizedSuperDelegate is LinearizedSuperBase {
    function next(bytes calldata payload) internal virtual override {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload); //~WARN: payable functions should not use `delegatecall` inside a loop
        require(ok);
    }
}

contract LinearizedSuperCaller is LinearizedSuperBase {
    function callNext(bytes calldata payload) internal {
        super.next(payload);
    }
}

contract DelegatecallLoop is
    ParentOrdinaryDelegatecall,
    ParentDelegatecallHelper,
    LinearizedSuperDelegate,
    LinearizedSuperCaller
{
    function next(bytes calldata payload) internal override(LinearizedSuperBase, LinearizedSuperDelegate) {
        super.next(payload);
    }

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

    modifier loopPlaceholder(uint256 iterations) {
        for (uint256 i = 0; i < iterations; ++i) {
            _;
        }
    }

    function payableModifierLoopPlaceholder(bytes calldata payload) external payable loopPlaceholder(3) {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload); //~WARN: payable functions should not use `delegatecall` inside a loop
        require(ok);
    }

    function payableLoopWithInternalDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            delegate(payloads[i]);
        }
    }

    function delegate(bytes calldata payload) internal {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload); //~WARN: payable functions should not use `delegatecall` inside a loop
        require(ok);
    }

    function payableInternalLoopWithDelegatecall(bytes[] calldata payloads) external payable {
        delegateInLoop(payloads);
    }

    function delegateInLoop(bytes[] calldata payloads) internal {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = target.delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
        }
    }

    function payableInternalDelegatecallOutsideLoop(bytes calldata payload) external payable {
        delegate(payload);
    }

    function payableLoopWithPublicDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            publicDelegate(payloads[i]);
        }
    }

    function publicDelegate(bytes calldata payload) public {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload); //~WARN: payable functions should not use `delegatecall` inside a loop
        require(ok);
    }

    function payableLoopWithSuperInternalDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            super.superDelegate(payloads[i]);
        }
    }

    function payableLoopWithLinearizedSuperDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            callNext(payloads[i]);
        }
    }

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

    function payableLoopWithInternalLibraryDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            DelegatecallLoopLib.helper(payloads[i]);
        }
    }

    function payableLoopCallsSafeOverload(uint256[] calldata values) external payable {
        for (uint256 i = 0; i < values.length; ++i) {
            overloaded(values[i]);
        }
    }

    function overloaded(uint256 value) internal pure returns (uint256) {
        return value;
    }

    function overloaded(bytes calldata payload) internal {
        address target = address(this);
        (bool ok,) = target.delegatecall(payload);
        require(ok);
    }

    function payableLoopCallsOrdinaryDelegatecall(
        OrdinaryDelegatecall callee,
        bytes[] calldata payloads
    ) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            require(callee.delegatecall(payloads[i]));
        }
    }

    function payableLoopCallsThisDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            require(this.delegatecall(payloads[i]));
        }
    }

    function payableLoopCallsSuperDelegatecall(bytes[] calldata payloads) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            require(super.delegatecall(payloads[i]));
        }
    }

    function payableLoopCallsConditionalDelegatecall(bytes[] calldata payloads, bool flag) external payable {
        address targetA = address(this);
        address targetB = address(0xBEEF);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = (flag ? targetA : targetB).delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
        }
    }

    function payableLoopCallsConditionalDelegatecallWithBinaryCondition(bytes[] calldata payloads) external payable {
        address targetA = address(this);
        address targetB = address(0xBEEF);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = (payloads[i].length != 0 ? targetA : targetB).delegatecall(payloads[i]); //~WARN: payable functions should not use `delegatecall` inside a loop
            require(ok);
        }
    }

    function payableLoopCallsConditionalOrdinaryDelegatecall(
        OrdinaryDelegatecall a,
        OrdinaryDelegatecall b,
        bytes[] calldata payloads,
        bool flag
    ) external payable {
        for (uint256 i = 0; i < payloads.length; ++i) {
            require((flag ? a : b).delegatecall(payloads[i]));
        }
    }

    function delegatecall(bytes calldata) public pure override returns (bool) {
        return true;
    }

    function nonPayableLoop(bytes[] calldata payloads) external {
        address target = address(this);
        for (uint256 i = 0; i < payloads.length; ++i) {
            (bool ok,) = target.delegatecall(payloads[i]);
            require(ok);
        }
    }
}
