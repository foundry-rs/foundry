// config: line_length = 100
library ArrayUtils {
    function map(uint256[] memory self, function(uint256) pure returns (uint256) f)
        internal
        pure
        returns (uint256[] memory r)
    {
        r = new uint256[](self.length);
        for (uint256 i = 0; i < self.length; i++) {
            r[i] = f(self[i]);
        }
    }

    function reduce(uint256[] memory self, function(uint256, uint256) pure returns (uint256) f)
        internal
        pure
        returns (uint256 r)
    {
        r = self[0];
        for (uint256 i = 1; i < self.length; i++) {
            r = f(r, self[i]);
        }
    }

    function range(uint256 length) internal pure returns (uint256[] memory r) {
        r = new uint256[](length);
        for (uint256 i = 0; i < r.length; i++) {
            r[i] = i;
        }
    }

    function _castToPure(function(bytes memory) internal view fnIn)
        returns (function(bytes memory) pure fnOut)
    {
        assembly {
            fnOut := fnIn
        }
    }
}
