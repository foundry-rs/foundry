library ArrayUtils {
    function map(uint256[] memory self, function (uint) pure returns (uint) f)
        internal
        pure
        returns (
            uint256[] memory r
        )
    {
        r = new uint[](self.length);
        for (uint i = 0; i < self.length; i++) {
            r[i] = f(self[i]);
        }
    }

    function reduce(
        uint256[] memory self,
        function (uint, uint) pure returns (uint) f
    ) internal pure returns (uint256 r) {
        r = self[0];
        for (uint i = 1; i < self.length; i++) {
            r = f(r, self[i]);
        }
    }

    function range(uint256 length) internal pure returns (uint256[] memory r) {
        r = new uint[](length);
        for (uint i = 0; i < r.length; i++) {
            r[i] = i;
        }
    }
}
