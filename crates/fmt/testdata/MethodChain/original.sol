// https://github.com/foundry-rs/foundry/issues/12344
contract MethodChain {
function shortChain() external {
value.first().second();
}

function longChain() external {
stdstore.target(address(eigenDAServiceManager)).sig("batchIdToBatchMetadataHash(uint32)").with_key(defaultBatchId).checked_write(CertV1Lib.hashBatchMetadata(batchMetadata));
}

function commentedChain() external {
factory() // preserve
.first().second();
}

function standaloneCommentChain() external {
factory()
// preserve
.first().second();
}
}
