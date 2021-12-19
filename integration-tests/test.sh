#!/bin/bash
set +x
set -e


# TODO: Add logic for running with all
REPO=$1
TESTDATA=testdata

ALLOWED_FAILURE_REPOS=("geb" "drai" "guni-lev", "multicall")
if [[ " ${ALLOWED_FAILURE_REPOS[*]} " =~ " ${REPO} " ]]; then
    export FORGE_ALLOW_FAILURE=1
fi

FORKED_REPOS=("drai" "guni-lev")
if [[ " ${FORKED_REPOS[*]} " =~ " ${REPO} " ]]; then
    FORK_ARGS="--rpc-url $ETH_RPC_URL"
fi


DIR=`pwd`
FORGE=${FORGE:-$DIR/../target/release/forge}

function runTests() {
    cd $TESTDATA/$1

    # run any installation step if needed
    make install || true

    # update the deps
    $FORGE update
    # always have the ffi flag turned on
    $FORGE test --ffi $FORK_ARGS

    cd -
}

runTests $REPO
