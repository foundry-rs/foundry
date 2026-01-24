// config: line_length = 120
// https://github.com/foundry-rs/foundry/issues/12399
contract StructFieldAccess {
    function a() external {
        bytes32 guid =
            _lzSend({
            _dstEid: dstEid,
            _message: message,
            _options: OptionsBuilder.newOptions().addExecutorLzReceiveOption({_gas: gasLimit, _value: 0}),
            _fee: MessagingFee({nativeFee: msg.value, lzTokenFee: 0}),
            _refundAddress: msg.sender
        }).guid;
    }

    function b() external view returns (uint256) {
        return _quote({
            _dstEid: dstEid,
            _message: message,
            _options: OptionsBuilder.newOptions().addExecutorLzReceiveOption({_gas: gasLimit, _value: 0}),
            _payInLzToken: false
        }).nativeFee;
    }

    // Simple cases
    function c() external {
        uint256 val = getData().value;
        bool flag = getStruct({param: 1}).isActive;
    }

    // Nested struct field access
    function d() external {
        uint256 nested = getOuter().inner.value;
    }

    // Chained calls with named args
    function e() external {
        bytes32 guid =
            _lzSend({
            _dstEid: dstEid,
            _message: message,
            _options: OptionsBuilder.newOptions().addExecutorLzReceiveOption({_gas: gasLimit, _value: 0}),
            _fee: MessagingFee({nativeFee: msg.value, lzTokenFee: 0}),
            _refundAddress: msg.sender
        }).wrap({wrapper: wrapperAddress, extraData: bytes("")}).guid;
    }
}
