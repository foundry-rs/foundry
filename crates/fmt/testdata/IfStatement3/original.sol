contract DanglingElse {
    function f(bool a, bool b) external pure returns (uint256) {
        if (a) { if (b) return 1; } else return 2;
        return 0;
    }

    function g(bool a, bool b) external pure returns (uint256) {
        if (a) { if (b) return 1; else return 7; } else return 2;
        return 0;
    }

    function h(bool a, bool b, bool c) external pure returns (uint256) {
        if (a) return 9;
        else if (b) { if (c) return 1; }
        else return 2;
        return 0;
    }

    function w(bool a, bool b, bool c) external pure returns (uint256) {
        if (a) { while (c) if (b) return 1; } else return 2;
        return 0;
    }

    function fr(bool a, bool b, uint256 n) external pure returns (uint256) {
        if (a) { for (uint256 i; i < n; ++i) if (b) return 1; } else return 2;
        return 0;
    }

    function ok(bool a, bool b) external pure returns (uint256) {
        if (a) { if (b) return 1; }
        return 0;
    }

    function nested(bool x, bool a, bool b) external pure returns (uint256) {
        if (x) { if (a) { if (b) return 1; } else return 2; }
        return 0;
    }

    function cIso(bool a, bool b) external pure returns (uint256) {
        // isolated comment
        if (a) { if (b) return 1; } else return 2;
        return 0;
    }

    function cMix(bool a, bool b) external pure returns (uint256) {
        if (a) { /* mixed */ if (b) return 1; } else return 2;
        return 0;
    }

    function cTrl(bool a, bool b) external pure returns (uint256) {
        if (a) { if (b) return 1; // trailing
        } else return 2;
        return 0;
    }
}
