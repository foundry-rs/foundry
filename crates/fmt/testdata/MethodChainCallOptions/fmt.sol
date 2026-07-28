// config: line_length = 60
contract MethodChainCallOptions {
    function formatCallOptions() external {
        target{value: amount}(shortArg)
            .firstLongMethod()
            .secondLongMethod();
    }

    function formatOverflowingCallOptions() external {
        target{
                value: v
            }(
                firstExtremelyLongArgument,
                secondExtremelyLongArgument
            )
            .firstLongMethod()
            .secondLongMethod();
    }
}
