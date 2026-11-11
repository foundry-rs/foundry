// config: line_length = 120
// config: single_line_statement_blocks = "multi"
// config: wrap_comments = false
contract C {
    function f() external {
        for (; c;) {
            x();
        }

        for (; c;) {
            x();
        }

        for (; c;) {
            x();
        }

        for (; c;) {
            x();
        }

        for (; c;) {
            x();
            y();
        }

        for (; c;) {
            /* comment */
            x();
        }
    }
}
