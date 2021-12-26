pragma solidity ^0.8.0;


interface RecursiveCallee {
	function recurseCall(uint256 neededDepth, uint256 depth) external returns (uint256);
	function recurseCreate(uint256 neededDepth, uint256 depth) external returns (uint256);
	function someCall() external;
	function negativeNum() external returns (int256);
}

contract RecursiveCall {
	event Depth(uint256 depth);
	event ChildDepth(uint256 childDepth);
	event CreatedChild(uint256 childDepth);
	Trace factory;

	constructor(address _factory) {
		factory = Trace(_factory);
	}

	function recurseCall(uint256 neededDepth, uint256 depth) public returns (uint256) {
		if (depth == neededDepth) {
			RecursiveCallee(address(this)).negativeNum();
			return neededDepth;
		}
		uint256 childDepth = RecursiveCallee(address(this)).recurseCall(neededDepth, depth + 1);
		emit ChildDepth(childDepth);
		RecursiveCallee(address(this)).someCall();
		emit Depth(depth);
		return depth;
	}

	function recurseCreate(uint256 neededDepth, uint256 depth) public returns (uint256) {
		if (depth == neededDepth) {
			return neededDepth;
		}
		RecursiveCall child = factory.create();
		emit CreatedChild(depth + 1);
		uint256 childDepth = child.recurseCreate(neededDepth, depth + 1);
		emit Depth(depth);
		return depth;
	}

	function someCall() public {}

	function negativeNum() public returns (int256) {
		return -1000000000;
	}
}

contract Trace {
	RecursiveCall first;

	function create() public returns (RecursiveCall) {
		if (address(first) == address(0)) {
			first = new RecursiveCall(address(this));
			return first;
		}
		return new RecursiveCall(address(this));
	}

	function recurseCall(uint256 neededDepth, uint256 depth) public returns (uint256) {
		if (address(first) == address(0)) {
			first = new RecursiveCall(address(this));
		}
		return first.recurseCall(neededDepth, depth);
	}

	function recurseCreate(uint256 neededDepth, uint256 depth) public returns (uint256) {
		if (address(first) == address(0)) {
			first = new RecursiveCall(address(this));
		}
		return first.recurseCreate(neededDepth, depth);
	}
}