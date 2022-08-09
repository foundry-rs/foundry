// config: quote_style = "preserve"
contract LiteralExpressions {
    function test() external {
        // bool literals
        true;
        false;
        /* comment1 */
        true; /* comment2 */
        // comment3
        false; // comment4

        // number literals
        1;
        123_000;
        1_2e345_678;
        -1;
        2e-10;
        // comment5
        /* comment6 */
        -1; /* comment7 */

        // hex number literals
        0x00;
        0x123_456;
        0x2eff_abde;

        // rational number literals
        0.1;
        1.3;
        2.5e1;

        // string literals
        "";
        "foobar";
        "foo" // comment8
        " bar";
        // comment9
        "\
some words"; /* comment10 */
        unicode"Hello 😃";

        // quoted strings
        'hello "world"';
        "hello 'world'";
        'hello \'world\'';
        "hello \"world\"";
        'hello \"world\"';
        "hello \'world\'";

        // hex literals
        hex"001122FF";
        hex'001122FF';
        hex"00112233" hex"44556677";

        // address literals
        0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
        // non checksummed address
        0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    }
}
