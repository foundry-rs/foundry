// config: hex_underscore = "remove"
contract HexLiteral {
    function test() external {
        hex"01230000";
        hex"01230000";
        hex"01230000";
        hex"";
        hex"6001600253";
    }
}
