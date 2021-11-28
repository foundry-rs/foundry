#!/bin/bash
set +x
set -e


# TODO: Add logic for running with all
REPO=$1
TESTDATA=testdata

ALLOWED_FAILURE_REPOS=("geb" "drai" "guni-lev")
if [[ " ${ALLOWED_FAILURE_REPOS[*]} " =~ " ${REPO} " ]]; then
    export FORGE_ALLOW_FAILURE=1
fi

DIR=`pwd`

function runTests() {
    cd $TESTDATA/$1

    # run any installation step if needed
    make install || true

    # update the deps
    $DIR/../target/debug/forge update
    # always have the ffi flag turned on
    $DIR/../target/debug/forge test --ffi

    cd -
}

runTests $REPO
