contract NestedNamedCallArgumentChain {
    function format(uint256 firstExtremelyLongValueName, uint256 secondExtremelyLongValueName) external {
        factory().item(innerFactory().anExtremelyLongMethodNameThatForcesTheNestedCalleeToWrap({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName})).update();
        factory().item(innerFactory().methodThatMakesThisNamedCallExactlyOneHundredAndTwentyOneCharsLongXX({value: firstExtremelyLongValueName})).update();
        factory().item(innerFactory().anExtremelyLongMethodNameThatForcesTheNestedCalleeToWrap({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName})) // keep this comment
            .update();
        factory().item(innerFactory().method({value: firstExtremelyLongValueName})).update();
        factory().item(innerFactory().anExtremelyLongMethodNameThatForcesTheNestedCalleeToWrap({enabled: true})).update();
        factory().item(innerFactory().anExtremelyLongMethodNameThatForcesTheNestedCalleeToWrap({value: firstExtremelyLongValueName + secondExtremelyLongValueName})).update();
    }
}
