// config: line_length = 60
contract ModifierDefinitions {
    modifier noParams() {}
    modifier oneParam(uint256 a) {}
    modifier twoParams(uint256 a, uint256 b) {}
    modifier threeParams(uint256 a, uint256 b, uint256 c) {}
    modifier fourParams(
        uint256 a,
        uint256 b,
        uint256 c,
        uint256 d
    ) {}
    modifier overridden() override(Base1, Base2) {}
    modifier inlineBlock()/* Inline explanation. */  {
        _;
    }
    modifier trailingLine(uint256 a) // Trailing explanation.
         {
        require(a > 0);
        _;
    }
    modifier isolatedLine()
        // Isolated explanation.
         {
        _;
    }
    modifier isolatedBlock()
        /* Block explanation. */
         {
        _;
    }
    modifier multiline()
        /* First line.
        Second line. */
         {
        _;
    }
    modifier attributed() virtual /* Virtual. */  {
        _;
    }
    constructor()/* Constructor explanation. */  {}
}
