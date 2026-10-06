// config: style = "tab"
contract MixedBlockComments {
	function condition(uint256 x) external pure returns (uint256) {
		if (
			x /* First line.
			  more text. */ > 0
		) {
			return 1;
		}
		return 0;
	}

	function expression(uint256 x) external pure returns (uint256) {
		return x /* Explanation.
				 Indented detail.

				 More detail. */ + 1;
	}

	function aligned(uint256 x) external pure returns (uint256) {
		return x /* Column A.
				    Column B.
				  Column C. */ + 1;
	}
}
