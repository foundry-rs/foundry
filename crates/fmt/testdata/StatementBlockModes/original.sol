contract C {
    function f() external {
        if (c) x();

        while (c) x();

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
