// config: line_length = 120
// config: single_line_statement_blocks = "multi"
// config: wrap_comments = false
contract C {
    function f() external {
        if (c) {
            x();
        }

        while (c) {
            x();
        }

        for (; c;) {
            x();
        }

        if (c) {
            x();
        }

        while (c) {
            x();
        }

        for (; c;) {
            x();
        }

        if (c) {
            x();
        }

        while (c) {
            x();
        }

        for (; c;) {
            x();
        }

        if (c) {
            x();
        }

        while (c) {
            x();
        }

        for (; c;) {
            x();
        }

        if (c) {
            x();
        }

        while (c) {
            x();
        }

        for (; c;) {
            x();
        }

        if (c) {
            x();
            y();
        }

        while (c) {
            x();
            y();
        }

        for (; c;) {
            x();
            y();
        }

        if (c) {
            /* comment */
            x();
        }

        while (c) {
            /* comment */
            x();
        }

        for (; c;) {
            /* comment */
            x();
        }

        for (uint256 i = 0; i < n; i++) {
            a[i] = b.get(i);
        }

        for (uint256 i = 0; i < n; i++) {
            a[i] = b.get(i);
        }

        for (uint256 i = 0; i < n; i++) {
            a[i] = b.get(i);
        }
    }
}
