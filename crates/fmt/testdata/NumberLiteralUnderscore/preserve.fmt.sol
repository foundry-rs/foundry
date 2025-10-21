// config: number_underscore = "preserve"
contract NumberLiteral {
    bytes4 internal constant HEX_NUM = 0x01ffc9a7;

    function test() external {
        1;
        123_000;
        // 1_2e345_678; // solar error: exponent too large
        -1;
        2e-10;
        0.1;
        1.3;
        2.5e1;
        1.23454;
        // 1.2e34_5_678; // solar error: exponent too large
        // 134411.2e34_5_678; // solar error: exponent too large
        // 13431.134112e34_135_678; // solar error: exponent too large
        13431.0134112;
        // 13431.134112e-139_3141340; // solar error: exponent too large
        // 00134411.200e0034_5_6780; // solar error: leading zeros are not allowed in integers
        // 013431.13411200e34_135_6780; // solar error: leading zeros are not allowed in integers
        // 00.1341120000; // solar error: leading zeros are not allowed in integers
        1.0;
        // 0013431.13411200e-00139_3141340; // solar error: leading zeros are not allowed in integers
        10_234e56;
        1234e56;
        10000;
        1_000;
        5_267.8268764263694426e18;
    }
}
