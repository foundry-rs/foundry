#!/bin/bash
set +x

# TODO: Add logic for running with all
REPO=$1
TESTDATA=testdata

DIR=`pwd`
FORGE_BIN=${FORGE_BIN:-forge}

function runTests() {
    cd $TESTDATA/$1

    # run any installation step if needed
    make install || true

    # update the deps
    $FORGE_BIN update
    # always have the ffi flag turned on
    $FORGE_BIN test --ffi

    cd -
}

runTests $REPO
