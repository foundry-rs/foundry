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
}
