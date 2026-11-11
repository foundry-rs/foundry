contract C {
    function f() external {
        for (; c;) x();

        for (; c;) { x(); }

        for (; c;) {
 x();
 }

        for (; c;)
 x();

        for (; c;) { x(); y(); }

        for (; c;) { /* comment */ x(); }
    }
}
