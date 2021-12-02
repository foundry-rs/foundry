pragma solidity ^0.8.0;


interface RecursiveCallee {
	function recurseCall(uint256 neededDepth, uint256 depth) external returns (uint256);
	function someCall() external;
}

contract RecursiveCall {
	event Depth(uint256 depth);
	function recurseCall(uint256 neededDepth, uint256 depth) public returns (uint256) {
		if (depth == neededDepth) {
			return neededDepth;
		}
		RecursiveCallee(address(this)).recurseCall(neededDepth, depth + 1);
		RecursiveCallee(address(this)).someCall();
		emit Depth(depth);
		return depth;
	}

	function someCall() public {
	}
}