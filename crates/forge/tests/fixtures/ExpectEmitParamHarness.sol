// SPDX-License-Identifier: UNLICENSED

// Harness contract are in a separate file so that the selectors cache can be populated on `forge b`.
contract EventEmitter {
    event SimpleEvent(uint256 indexed a, uint256 indexed b, uint256 c);
    event ComplexEvent(address indexed sender, uint256 indexed id, bytes data);

    function emitSimple(uint256 a, uint256 b, uint256 c) public {
        emit SimpleEvent(a, b, c);
    }

    function emitComplex(address sender, uint256 id, bytes memory data) public {
        emit ComplexEvent(sender, id, data);
    }

    function emitSimpleMultipleTimes(
        uint256 a,
        uint256 b,
        uint256 c,
        uint256 times
    ) public {
        for (uint256 i = 0; i < times; i++) {
            emit SimpleEvent(a, b, c);
        }
    }
}

contract SelectiveEmitter {
    event TestEvent(uint256 indexed a, uint256 indexed b, uint256 c);

    function emitEvent(uint256 a, uint256 b, uint256 c) public {
        emit TestEvent(a, b, c);
    }
}

contract ParamNumberingEmitter {
    // Event with 2 indexed and 3 non-indexed parameters
    event MixedEventNumbering(
        uint256 indexed param0, // param 0 (indexed)
        address indexed param1, // param 1 (indexed)
        uint256 param2, // param 2 (non-indexed)
        uint256 param3, // param 3 (non-indexed)
        address param4 // param 4 (non-indexed)
    );

    function emitEvent(
        uint256 p0,
        address p1,
        uint256 p2,
        uint256 p3,
        address p4
    ) public {
        emit MixedEventNumbering(p0, p1, p2, p3, p4);
    }
}

contract AnonymousEmitter {
    // Anonymous event with indexed parameter
    event AnonymousIndexed(uint256 indexed a, uint256 b, address c) anonymous;

    function emitAnonymousIndexed(uint256 a, uint256 b, address c) public {
        emit AnonymousIndexed(a, b, c);
    }
}

contract ManyParamsEmitter {
    // Event with many non-indexed parameters to trigger raw data display
    event ManyParams(uint256 a, uint256 b, uint256 c, uint256 d, uint256 e);

    function emitManyParams(
        uint256 a,
        uint256 b,
        uint256 c,
        uint256 d,
        uint256 e
    ) public {
        emit ManyParams(a, b, c, d, e);
    }
}
