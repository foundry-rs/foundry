// config: line_length = 40
// config: single_line_statement_blocks = "single"
// config: wrap_comments = false
contract C {
    function f() external {
        for (;;) {
            x = uint256(
                11111111111111111111111111111111111111 /* c */
            ) + 1;
        }

        if (c) {
            x = uint256(
                11111111111111111111111111111111111111 /* c */
            ) + 1;
        }

        while (c) {
            x = uint256(
                11111111111111111111111111111111111111 /* c */
            ) + 1;
        }
    }
}
