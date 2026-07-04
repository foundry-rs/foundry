//@compile-flags: --only-lint incorrect-using-for
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `incorrect-using-for`: a `using L for T` directive whose library has no function
// accepting `T` as its bound first parameter attaches nothing, so the directive is dead weight
// and probably a typo. Attachment follows the type checker, implicit conversions included: a
// `uint8` binds to a `uint256` parameter and a derived contract to a base parameter. Directives
// that attach at least one function, `using L for *` and the braced form (already checked by
// the compiler) stay clean.

struct Point {
    uint256 x;
}

library PointLib {
    function norm(Point storage p) internal view returns (uint256) {
        return p.x;
    }
}

library StringLib {
    function shout(string memory s) internal pure returns (string memory) {
        return s;
    }
}

library WideLib {
    function double(uint256 v) internal pure returns (uint256) {
        return v * 2;
    }
}

// One applicable function is enough for the directive to be useful.
library MixedLib {
    function onPoint(Point storage p) internal view returns (uint256) {
        return p.x;
    }

    function onString(string memory s) internal pure returns (uint256) {
        return bytes(s).length;
    }
}

contract Base {}

contract Derived is Base {}

library BaseLib {
    function tag(Base) internal pure returns (uint256) {
        return 1;
    }
}

using PointLib for Point global;

// No function of StringLib accepts a Point: the file-level directive attaches nothing.
using StringLib for Point; //~NOTE: `using ... for` names a library

// A uint8 binds to the uint256 parameter through implicit widening.
using WideLib for uint8;

contract UsesDirectives {
    // No function of StringLib accepts a uint256: the contract-level directive attaches
    // nothing.
    using StringLib for uint256; //~NOTE: `using ... for` names a library

    using MixedLib for Point;

    // A Derived value binds to the Base parameter through implicit conversion.
    using BaseLib for Derived;

    // The star form attaches every function of the library: out of scope.
    using StringLib for *;

    // The braced form is type-checked by the compiler, a mismatch does not compile.
    using {WideLib.double} for uint256;

    Point internal point;

    function useAttachments(Derived d, uint8 small, string memory s) internal view returns (uint256) {
        return point.norm() + point.onPoint() + d.tag() + small.double() + bytes(s.shout()).length;
    }
}
