interface FunctionInterfaces {
    function noParamsNoModifiersNoReturns();

    function oneParam(uint x);

    function oneModifier() modifier1;

    function oneReturn() returns(uint y1);

    // function prefix
    function withComments( // function name postfix
    // x1 prefix
    uint256 x1, // x1 postfix

        // x2 prefix
            uint256 x2, // x2 postfix
                // x2 postfix2
        /*
            multi-line x3 prefix
        */
        uint256 x3 // x3 postfix

    )
            // pure prefix
                pure // pure postfix
        // modifier1 prefix
        modifier1 // modifier1 postfix
    // public prefix
    public // public postfix
        // modifier2 prefix
        modifier2 /*
                    mutliline modifier2 postfix
                    */
            // modifier3 prefix
            modifier3 // modifier3 postfix
        returns (
            // y1 prefix
            uint256 y1, // y1 postfix
            // y2 prefix
            uint256 y2, // y2 postfix
            // y3 prefix
            uint256 y3 // y3 postfix
        ); // function postfix

    /*//////////////////////////////////////////////////////////////////////////
                                    TEST
    //////////////////////////////////////////////////////////////////////////*/
    function manyParams(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10);

    function manyModifiers() modifier1() modifier2() modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10;

    function manyReturns() returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10);

    function someParamsSomeModifiers(uint x1, uint x2, uint x3) modifier1() modifier2 modifier3;

    function someParamsSomeReturns(uint x1, uint x2, uint x3) returns(uint y1, uint y2, uint y3);

    function someModifiersSomeReturns() modifier1 modifier2 modifier3 returns(uint y1, uint y2, uint y3);

    function someParamSomeModifiersSomeReturns(uint x1, uint x2, uint x3) modifier1 modifier2 modifier3 returns(uint y1, uint y2, uint y3);

    function someParamsManyModifiers(uint x1, uint x2, uint x3) modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10;

    function someParamsManyReturns(uint x1, uint x2, uint x3) returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10);

    function manyParamsSomeModifiers(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) modifier1 modifier2 modifier3;

    function manyParamsSomeReturns(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) returns(uint y1, uint y2, uint y3);

    function manyParamsManyModifiers(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10;

    function manyParamsManyReturns(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10);

    function manyParamsManyModifiersManyReturns(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10 returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10);

    function modifierOrderCorrect01() public view virtual override modifier1 modifier2 returns(uint);

    function modifierOrderCorrect02() private pure virtual modifier1 modifier2 returns(string);

    function modifierOrderCorrect03() external payable override modifier1 modifier2 returns(address);

    function modifierOrderCorrect04() internal virtual override modifier1 modifier2 returns(uint);

    function modifierOrderIncorrect01() public modifier1 modifier2 override virtual view returns(uint);

    function modifierOrderIncorrect02() virtual modifier1 external modifier2 override returns(uint);

    function modifierOrderIncorrect03() modifier1 pure internal virtual modifier2 returns(uint);

    function modifierOrderIncorrect04() override modifier1 payable external modifier2 returns(uint);
}

contract FunctionDefinitions {
    function () external {}
    fallback () external {}

    function () external payable {}
    fallback () external payable {}
    receive () external payable {}

    function noParamsNoModifiersNoReturns() {
        a = 1;
    }

    function oneParam(uint x) {
        a = 1;
    }

    function oneModifier() modifier1 {
        a = 1;
    }

    function oneReturn() returns(uint y1) {
        a = 1;
    }

    function manyParams(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) {
        a = 1;
    }

    function manyModifiers() modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10 {
        a = 1;
    }

    function manyReturns() returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10) {
        a = 1;
    }

    function someParamsSomeModifiers(uint x1, uint x2, uint x3) modifier1 modifier2 modifier3 {
        a = 1;
    }

    function someParamsSomeReturns(uint x1, uint x2, uint x3) returns(uint y1, uint y2, uint y3) {
        a = 1;
    }

    function someModifiersSomeReturns() modifier1 modifier2 modifier3 returns(uint y1, uint y2, uint y3) {
        a = 1;
    }

    function someParamSomeModifiersSomeReturns(uint x1, uint x2, uint x3) modifier1 modifier2 modifier3 returns(uint y1, uint y2, uint y3) {
        a = 1;
    }

    function someParamsManyModifiers(uint x1, uint x2, uint x3) modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10 {
        a = 1;
    }

    function someParamsManyReturns(uint x1, uint x2, uint x3) returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10) {
        a = 1;
    }

    function manyParamsSomeModifiers(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) modifier1 modifier2 modifier3 {
        a = 1;
    }

    function manyParamsSomeReturns(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) returns(uint y1, uint y2, uint y3) {
        a = 1;
    }

    function manyParamsManyModifiers(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10 public {
        a = 1;
    }

    function manyParamsManyReturns(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10) {
        a = 1;
    }

    function manyParamsManyModifiersManyReturns(uint x1, uint x2, uint x3, uint x4, uint x5, uint x6, uint x7, uint x8, uint x9, uint x10) modifier1 modifier2 modifier3 modifier4 modifier5 modifier6 modifier7 modifier8 modifier9 modifier10 returns(uint y1, uint y2, uint y3, uint y4, uint y5, uint y6, uint y7, uint y8, uint y9, uint y10) {
        a = 1;
    }

    function modifierOrderCorrect01() public view virtual override modifier1 modifier2 returns(uint) {
        a = 1;
    }

    function modifierOrderCorrect02() private pure virtual modifier1 modifier2 returns(string) {
        a = 1;
    }

    function modifierOrderCorrect03() external payable override modifier1 modifier2 returns(address) {
        a = 1;
    }

    function modifierOrderCorrect04() internal virtual override modifier1 modifier2 returns(uint) {
        a = 1;
    }

    function modifierOrderIncorrect01() public modifier1 modifier2 override virtual view returns(uint) {
        a = 1;
    }

    function modifierOrderIncorrect02() virtual modifier1 external modifier2 override returns(uint) {
        a = 1;
    }

    function modifierOrderIncorrect03() modifier1 pure internal virtual modifier2 returns(uint) {
        a = 1;
    }

    function modifierOrderIncorrect04() override modifier1 payable external modifier2 returns(uint) {
        a = 1;
    }

    fallback() external payable virtual {}
    receive() external payable virtual {}
}

contract FunctionOverrides is FunctionInterfaces, FunctionDefinitions {
    function noParamsNoModifiersNoReturns() override {
        a = 1;
    }

    function oneParam(uint256 x) override(FunctionInterfaces, FunctionDefinitions, SomeOtherFunctionContract, SomeImport.AndAnotherFunctionContract) {
        a = 1;
    }
}

