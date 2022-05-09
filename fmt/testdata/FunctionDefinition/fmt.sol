interface FunctionInterfaces {
    function noParamsNoModifiersNoReturns();

    function oneParam(uint256 x);

    function oneModifier() modifier1;

    function oneReturn() returns (uint256 y1);

    function manyParams(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    );

    function manyModifiers()
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10;

    function manyReturns()
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        );

    function someParamsSomeModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3
    ) modifier1 modifier2 modifier3;

    function someParamsSomeReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        );

    function someModifiersSomeReturns()
        modifier1
        modifier2
        modifier3
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        );

    function someParamSomeModifiersSomeReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        modifier1
        modifier2
        modifier3
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        );

    function someParamsManyModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10;

    function someParamsManyReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        );

    function manyParamsSomeModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    ) modifier1 modifier2 modifier3;

    function manyParamssomeReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        );

    function manyParamsManyModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10;

    function manyParamsManyReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        );

    function manyParamsManyModifiersManyReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        );

    function modifierOrderCorrect01()
        public
        view
        virtual
        override
        modifier1
        modifier2
        returns (uint256);

    function modifierOrderCorrect02()
        private
        pure
        virtual
        modifier1
        modifier2
        returns (string);

    function modifierOrderCorrect03()
        external
        payable
        override
        modifier1
        modifier2
        returns (address);

    function modifierOrderCorrect04()
        internal
        virtual
        override
        modifier1
        modifier2
        returns (uint256);

    function modifierOrderIncorrect01()
        public
        view
        virtual
        override
        modifier1
        modifier2
        returns (uint256);

    function modifierOrderIncorrect02()
        external
        virtual
        override
        modifier1
        modifier2
        returns (uint256);

    function modifierOrderIncorrect03()
        internal
        pure
        virtual
        modifier1
        modifier2
        returns (uint256);

    function modifierOrderIncorrect04()
        external
        payable
        override
        modifier1
        modifier2
        returns (uint256);
}

contract FunctionDefinitions {
    function() external {}

    fallback() external {}

    function() external payable {}

    fallback() external payable {}

    receive() external payable {}

    function noParamsNoModifiersNoReturns() {
        a = 1;
    }

    function oneParam(uint256 x) {
        a = 1;
    }

    function oneModifier() modifier1 {
        a = 1;
    }

    function oneReturn() returns (uint256 y1) {
        a = 1;
    }

    function manyParams(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    ) {
        a = 1;
    }

    function manyModifiers()
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10
    {
        a = 1;
    }

    function manyReturns()
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        )
    {
        a = 1;
    }

    function someParamsSomeModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3
    ) modifier1 modifier2 modifier3 {
        a = 1;
    }

    function someParamsSomeReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        )
    {
        a = 1;
    }

    function someModifiersSomeReturns()
        modifier1
        modifier2
        modifier3
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        )
    {
        a = 1;
    }

    function someParamSomeModifiersSomeReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        modifier1
        modifier2
        modifier3
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        )
    {
        a = 1;
    }

    function someParamsManyModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10
    {
        a = 1;
    }

    function someParamsManyReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        )
    {
        a = 1;
    }

    function manyParamsSomeModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    ) modifier1 modifier2 modifier3 {
        a = 1;
    }

    function manyParamssomeReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3
        )
    {
        a = 1;
    }

    function manyParamsManyModifiers(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        public
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10
    {
        a = 1;
    }

    function manyParamsManyReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        )
    {
        a = 1;
    }

    function manyParamsManyModifiersManyReturns(
        uint256 x1,
        uint256 x2,
        uint256 x3,
        uint256 x4,
        uint256 x5,
        uint256 x6,
        uint256 x7,
        uint256 x8,
        uint256 x9,
        uint256 x10
    )
        modifier1
        modifier2
        modifier3
        modifier4
        modifier5
        modifier6
        modifier7
        modifier8
        modifier9
        modifier10
        returns (
            uint256 y1,
            uint256 y2,
            uint256 y3,
            uint256 y4,
            uint256 y5,
            uint256 y6,
            uint256 y7,
            uint256 y8,
            uint256 y9,
            uint256 y10
        )
    {
        a = 1;
    }

    function modifierOrderCorrect01()
        public
        view
        virtual
        override
        modifier1
        modifier2
        returns (uint256)
    {
        a = 1;
    }

    function modifierOrderCorrect02()
        private
        pure
        virtual
        modifier1
        modifier2
        returns (string)
    {
        a = 1;
    }

    function modifierOrderCorrect03()
        external
        payable
        override
        modifier1
        modifier2
        returns (address)
    {
        a = 1;
    }

    function modifierOrderCorrect04()
        internal
        virtual
        override
        modifier1
        modifier2
        returns (uint256)
    {
        a = 1;
    }

    function modifierOrderIncorrect01()
        public
        view
        virtual
        override
        modifier1
        modifier2
        returns (uint256)
    {
        a = 1;
    }

    function modifierOrderIncorrect02()
        external
        virtual
        override
        modifier1
        modifier2
        returns (uint256)
    {
        a = 1;
    }

    function modifierOrderIncorrect03()
        internal
        pure
        virtual
        modifier1
        modifier2
        returns (uint256)
    {
        a = 1;
    }

    function modifierOrderIncorrect04()
        external
        payable
        override
        modifier1
        modifier2
        returns (uint256)
    {
        a = 1;
    }

    fallback() external payable virtual {}

    receive() external payable virtual {}
}
