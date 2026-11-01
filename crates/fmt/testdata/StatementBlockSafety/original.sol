contract C {
    function f() external {
        for (;;) { uint256 x; }

        for (;;) { (uint256 x, uint256 y) = g(); }

        while (c) { uint256 x; }

        while (c) { (uint256 x, uint256 y) = g(); }

        if (c) { uint256 x; }

        if (c) { (uint256 x, uint256 y) = g(); }

        for (;;) while (c) { uint256 x; }

        for (;;) while (c) { (uint256 x, uint256 y) = g(); }

        for (;;) g(/* c */ 1);

        for (;;) { { break; } }

        for (;;) { { { break; } } }

        if (c) g(/* c */ 1);

        if (c) { { break; } }

        if (c) { { { break; } } }

        while (c) g(/* c */ 1);

        while (c) { { break; } }

        while (c) { { { break; } } }

        for (;;) if (/* c */ c) break;

        for (;;) { if (/* c */ c) break; }

        if (a) {
 if (b) {
 x();
 }
 }

        while (a) {
 if (b) {
 x();
 }
 }

        for (uint256 i; i < n; ++i) {
 if (b) {
 x();
 }
 }

        if (a) while (b) for (;;) { if (c) return 1; } else return 2;

        for (/* init */ uint i=0; /* cond */ i<n; /* next */ ++i) { unchecked { x(); } }

        for (;;) { x(); // after
 }

        for (;;) /* header ) */
 { x(); }

        for (;;) { /* before */ x(); /* after */ }

        for (;;) { while (b) { if (c) x(); } }

        for (;a;) { while (b) { if (c) x(); } }

        for (;;x()) { while (b) { if (c) x(); } }

        for (x();;) { while (b) { if (c) x(); } }
    }
}
