pragma solidity ^0.8.8;

contract ForStatement {
    function test() external {
        for
    (uint256 i1
        ; i1 < 10;      i1++)
    {
             i1++;
            }

        uint256 i2;
        for(++i2;i2<10;i2++)
        
        {}

        uint256 veryLongVariableName = 1000;
        for ( uint256 i3; i3 < 10
        && veryLongVariableName>999 &&      veryLongVariableName< 1001
        ; i3++)
        { i3 ++ ; }

        for (type(uint256).min;;) {}

        for (;;) { "test" ; }

        for (uint256 i4; i4< 10; i4++) i4++;

        for (uint256 i5; ;)
            for (uint256 i6 = 10; i6 > i5; i6--)
                i5++;
    }
}