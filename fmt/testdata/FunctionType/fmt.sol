// config: line-length=90
library ArrayUtils {
    function map(uint256[] memory self, function (uint) pure returns (uint) f)
        internal
        pure
        returns (uint256[] memory r)
    {}

    function reduce(uint256[] memory self, function (uint, uint) pure returns (uint) f)
        internal
        pure
        returns (uint256 r)
    {}

    function range(uint256 length) internal pure returns (uint256[] memory r) {}
}
