// config: line_length = 80
// config: bracket_spacing = true
contract Route {
    function f() external {
        _OrderToRouteType[BasicOrderType.ETH_TO_ERC1155_PARTIAL_RESTRICTED] =
        BasicOrderRouteType.ETH_TO_ERC1155;
    }
}

contract Boolean {
    function f() external {
        context.expectations.ineligibleFailures[uint256(ineligibleFailure)] =
        true;
    }
}

contract AlreadyStable {
    function f() external {
        _OrderToRouteType[
            BasicOrderType.ERC20_TO_ERC1155_PARTIAL_RESTRICTED
        ] = BasicOrderRouteType.ERC20_TO_ERC1155;
    }
}

contract Commented {
    function f() external {
        _OrderToRouteType[BasicOrderType.ETH_TO_ERC1155_PARTIAL_RESTRICTED] = /* preserve */
            BasicOrderRouteType.ETH_TO_ERC1155;
    }
}
