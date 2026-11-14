contract C {
    function f() external payable returns (Execution[] memory executions) {
        executions = zone.executeMatchAdvancedOrders{ value: msg.value }(
            seaportAddress,
            orders,
            criteriaResolvers,
            fulfillments
        );
    }
}
