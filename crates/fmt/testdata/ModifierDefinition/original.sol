contract ModifierDefinitions {
    modifier noParams() {}
    modifier oneParam(uint a) {}
    modifier twoParams(uint a,uint b) {}
    modifier threeParams(uint a,uint b   ,uint c) {}
    modifier fourParams(uint a,uint b   ,uint c, uint d) {}
    modifier overridden (
    ) override ( Base1 , Base2) {}
    modifier inlineBlock() /* Inline explanation. */ { _; }
    modifier trailingLine(uint a) // Trailing explanation.
    { require(a > 0); _; }
    modifier isolatedLine()
    // Isolated explanation.
    { _; }
    modifier isolatedBlock()
    /* Block explanation. */
    { _; }
    modifier multiline() /* First line.
    Second line. */ { _; }
    modifier attributed() virtual /* Virtual. */ { _; }
    constructor() /* Constructor explanation. */ {}
}
