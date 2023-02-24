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
}
