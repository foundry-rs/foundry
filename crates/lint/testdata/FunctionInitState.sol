//@compile-flags: --only-lint function-init-state
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `function-init-state`: state variable initializers run at construction, before the
// constructor body, in base-to-derived order. An initializer that reads another non-constant
// state variable or calls a non-pure function observes that partial state (default values,
// surprising ordering), so the computed value is rarely the one intended. References to
// constants, calls to pure functions and plain literals are fine, and assignments made inside
// the constructor body are out of scope.

uint256 constant FILE_LEVEL = 10;

contract Base {
    function baseValue() internal view returns (uint256) {
        return address(this).balance;
    }

    function basePure(uint256 value) internal pure returns (uint256) {
        return value + 1;
    }
}

contract Oracle {
    uint256 public price = 3;
}

contract OracleChild is Oracle {}

contract OverrideBase {
    function maybePure() public view virtual returns (uint256) {
        return address(this).balance;
    }
}

contract OverrideChild is OverrideBase {
    function maybePure() public pure override returns (uint256) {
        return 1;
    }

    // `super` bypasses the local pure override and resolves to the base's view declaration
    uint256 public fromSuperCall = super.maybePure(); //~NOTE: state variable initializer
}

library Clock {
    function stamp(uint256 value) internal view returns (uint256) {
        return value + block.timestamp;
    }

    function pureStamp(uint256 value) internal pure returns (uint256) {
        return value + 1;
    }
}

library Consts {
    uint256 internal constant LC = 3;
}

contract InitFromFunction is Base {
    uint256 internal seed = 77;

    uint256 public fromNonPure = set(); //~NOTE: state variable initializer

    uint256 public fromView = read(); //~NOTE: state variable initializer

    uint256 public fromStateRead = seed + 1; //~NOTE: state variable initializer

    // the state reference hides in the argument of a pure call
    uint256 public fromPureWithStateArg = double(seed); //~NOTE: state variable initializer

    uint256 public fromInherited = baseValue(); //~NOTE: state variable initializer

    // the qualified form of the same inherited view call
    uint256 public fromQualifiedInherited = Base.baseValue(); //~NOTE: state variable initializer

    // an external call to another contract's getter reads that contract's state
    uint256 public fromExternalGetter = Oracle(address(0x1234)).price(); //~NOTE: state variable initializer

    // the getter is inherited: it is not among the child's own items
    uint256 public fromInheritedGetter = OracleChild(address(0x1234)).price(); //~NOTE: state variable initializer

    using Clock for uint256;

    // a `using for` call binds the library function to the value type
    uint256 public fromUsingView = uint256(5).stamp(); //~NOTE: state variable initializer

    uint256 public fromUsingPure = uint256(5).pureStamp();

    // only the overload the call dispatches to matters: `mix(1)` is the pure one
    uint256 public fromPureOverload = mix(1);

    uint256 public fromViewOverload = mix(1, 2); //~NOTE: state variable initializer

    // a same-arity overload pair differing by parameter type: `typed(1)` is the pure one
    uint256 public fromPureSameArityOverload = typed(1);

    uint256 public fromViewSameArityOverload = typed("s"); //~NOTE: state variable initializer

    // dispatch selects the child's pure override, not the view base declaration behind it
    uint256 public fromPureOverride = OverrideChild(address(0x1234)).maybePure();

    // typed through the base, the resolved declaration is the base's own view one
    uint256 public fromBaseTypedOverride = OverrideBase(address(0x1234)).maybePure(); //~NOTE: state variable initializer

    // a call through a function pointer stored in state reads that variable
    uint256 public fromPointerCall = fnPtr(); //~NOTE: state variable initializer

    function() internal pure returns (uint256) internal fnPtr = noState;

    uint256 public immutable fromNonPureImmutable = set(); //~NOTE: state variable initializer

    uint256 public fromLiteral = 5;

    uint256 public constant LOCAL_CONSTANT = 7;

    uint256 public fromConstant = LOCAL_CONSTANT + 1;

    uint256 public fromFileConstant = FILE_LEVEL * 2;

    uint256 public fromLibConstant = Consts.LC + 1;

    uint256 public fromQualifiedPure = Base.basePure(21);

    uint256 public fromPure = double(21);

    uint256 internal assignedInConstructor;

    constructor() {
        // constructor-body assignments run after every initializer: out of scope
        assignedInConstructor = set();
    }

    function set() internal returns (uint256) {
        seed = 78;
        return seed;
    }

    function read() internal view returns (uint256) {
        return seed;
    }

    function double(uint256 value) internal pure returns (uint256) {
        return value * 2;
    }

    function mix(uint256 x) internal pure returns (uint256) {
        return x + 1;
    }

    function mix(uint256 x, uint256 y) internal view returns (uint256) {
        return x + y + seed;
    }

    function typed(uint256 x) internal pure returns (uint256) {
        return x + 1;
    }

    function typed(string memory) internal view returns (uint256) {
        return seed;
    }

    function noState() internal pure returns (uint256) {
        return 4;
    }

    function localIsFine() external view returns (uint256) {
        // a local variable initialized from state inside a function body: out of scope
        uint256 local = seed + read();
        return local;
    }
}
