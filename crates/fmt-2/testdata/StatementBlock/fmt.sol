contract Contract {
    function test() {
        unchecked {
            a += 1;
        }

        unchecked {
            a += 1;
        }
        2 + 2;

        unchecked {
            a += 1;
        }
        unchecked {}

        1 + 1;
    }
}
