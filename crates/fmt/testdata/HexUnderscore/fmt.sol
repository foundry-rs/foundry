contract HexLiteral {
    function test() external {
        hex"0123_0000";
        hex"01230000";
        hex"0123_00_00";
        hex"";
        hex"6001_6002_53";
    }
}
