// config: bracket_spacing = true
contract TestContract {
    function aLongerTestFunctionName(uint256 input)
        public
        view
        returns (uint256 num)
    {
        (, uint256 second) = (1, 2);
        (uint256 listItem001) = 1;
        (uint256 listItem002, uint256 listItem003) = (10, 20);
        (uint256 listItem004, uint256 listItem005, uint256 listItem006) =
            (10, 20, 30);
        (
            uint256 listItem007,
            uint256 listItem008,
            uint256 listItem009,
            uint256 listItem010
        ) = (10, 20, 30, 40);
        return 1;
    }

    function test() external {
        uint256 value = map[key];
        uint256 allowed = allowance[from][msg.sender];
        allowance[from][msg.sender] = allowed;
    }

    function test_longAssignements() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "bash";
        inputs[1] = "-c";
        inputs[2] =
            "echo -n 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000966666920776f726b730000000000000000000000000000000000000000000000";
    }

    function test_stringConcatenation() public {
        string memory strConcat = "0," "11579208923731619542357098500868790785,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    }
}
