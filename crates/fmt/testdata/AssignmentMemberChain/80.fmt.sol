// config: line_length = 80
// config: bracket_spacing = true
function f() {
    if (outputSelection.actualEvents) {
        jsonOut =
            context.actualEvents.serializeTransferLogs("root", "actualEvents");
    }
}

contract Nested {
    function f() external {
        if (a) {
            if (b) {
                if (c) {
                    if (d) {
                        newOffer[j] =
                            orders[orderInsertionIndex].parameters.offer[j - 1];
                    }
                }
            }
        }
    }
}

contract Initializer {
    function f() external {
        for (uint256 i; i < 2; i++) {
            AdvancedOrder memory order =
                context.executionState.orders[fulfillmentComponent.orderIndex];
        }
    }
}

contract Commented {
    function f() external {
        if (a) {
            if (b) {
                if (c) {
                    if (d) {
                        newOf[j] = /* preserve */
                            orders[orderInsertionIndex].parameters.offer[j - 1];
                    }
                }
            }
        }
    }
}

contract AlreadyStable {
    function f() external {
        request.seaport = ConsiderationInterface(
            0x00000000000000ADc04C56Bf30aC9d3c0aAF14dC
        );
        if (a) {
            if (b) {
                consideration = orderToExecute.receivedItems[
                    potentialCandidate.itemIndex
                ];
            }
        }
    }
}
