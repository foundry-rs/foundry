contract ModifierDefinitions {
    modifier noParams() {}
    modifier oneParam(uint a) {}
    modifier twoParams(uint a,uint b) {}
    modifier threeParams(uint a,uint b   ,uint c) {}
    modifier fourParams(uint a,uint b   ,uint c, uint d) {}
    modifier overridden (
    ) override ( Base1 , Base2) {}
}
