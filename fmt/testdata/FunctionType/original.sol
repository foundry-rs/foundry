library ArrayUtils {
    function map(uint[] memory self, function (uint) pure returns (uint) f)
        internal
        pure
        returns (
            uint[] memory r
        ) {}

    function reduce(
        uint[] memory self,
        function (uint, uint) pure returns (uint) f
    ) internal pure returns (uint256 r) {}

    function range(uint256 length) internal pure returns (uint[] memory r) {}
}
