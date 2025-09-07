// config: hex_underscore = "bytes"
contract HexLiteral {
    function test() external {
        hex"01_23_00_00";
        hex"01_23_00_00";
        hex"01_23_00_00";
        hex"";
        hex"60_01_60_02_53";
    }
}
