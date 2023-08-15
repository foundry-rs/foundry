// Repros of fmt issues

// https://github.com/foundry-rs/foundry/issues/4403
function errorIdentifier() {
    bytes memory error = bytes("");
    if (error.length > 0) {}
}
