// config: line_length = 60
contract MethodChainCallOptions {
    function formatCallOptions() external {
        target{value: amount}(shortArg)
            .firstLongMethod()
            .secondLongMethod();
    }
}
