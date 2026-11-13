// config: line_length = 120
// config: single_line_statement_blocks = "single"
// config: wrap_comments = false
contract C {
    function f() external {
        for (;;) {
            uint256 x;
        }

        for (;;) {
            (uint256 x, uint256 y) = g();
        }

        for (;;) {
            while (c) {
                (uint256 x, uint256 y) = g();
            }
        }

        for (;;) {
            g( /* c */
                1
            );
        }

        for (;;) {
            {
                break;
            }
        }

        for (;;) {
            if ( /* c */
                c
            ) {
                break;
            }
        }

        for (uint256 i; i < n; ++i) if (b) x();

        if (a) {
            while (b) {
                for (;;) if (c) return 1;
            }
        } else {
            return 2;
        }

        for (/* init */ uint256 i = 0; /* cond */ i < n; /* next */ ++i) {
            unchecked {
                x();
            }
        }

        for (;;) x(); // after

        for (;;)  /* header ) */
        {
            x();
        }

        for (;;) {
            /* before */
            x(); /* after */
        }

        for (;;) while (b) if (c) x();

        for (; a;) while (b) if (c) x();

        for (;; x()) while (b) if (c) x();

        for (x();;) while (b) if (c) x();
    }
}
