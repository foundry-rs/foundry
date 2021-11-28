#!/bin/bash
set +x

# TODO: Add logic for running with all
REPO=$1
TESTDATA=testdata

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
