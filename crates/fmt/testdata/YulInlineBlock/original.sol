contract YulInlineBlock {
    function switchStmt() external view {
        assembly { switch 0 case 0 {} default {} }
    }

    function forStmt() external view {
        assembly { for {} 1 { pop(sload(0)) } { break continue } }
    }

    function nestedIfWithTwoStmts() external view {
        assembly { if 1 { pop(sload(0)) pop(sload(1)) } }
    }

    function stillInlined() external view {
        assembly { pop(sload(0)) }
    }

    function ifStillInlined() external view {
        assembly { if 1 { pop(sload(0)) } }
    }
}
