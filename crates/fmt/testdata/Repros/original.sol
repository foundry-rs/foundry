// Repros of fmt issues

// https://github.com/foundry-rs/foundry/issues/4403
function errorIdentifier() {
    bytes memory error = bytes("");
    if (error.length > 0) {}
}

// https://github.com/foundry-rs/foundry/issues/7549
function one() external {
    this.other({
        data: abi.encodeCall(
            this.other,
            (
                "bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla bla"
            )
            )
    });
}
