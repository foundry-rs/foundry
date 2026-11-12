// config: line_length = 120
// config: single_line_statement_blocks = "single"
// config: wrap_comments = false
contract C {
    function f() external {
        for (; c;) x();

        for (; c;) x();

        for (; c;) x();

        for (; c;) x();

        for (; c;) {
            x();
            y();
        }

        for (; c;) {
            /* comment */
            x();
        }

        if (c) if (d) x();

        if (c) if (d) x();

        while (c) x(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, -1);

        while (c) x = a + b;
    }
}
