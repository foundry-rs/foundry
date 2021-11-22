#!/bin/bash
# Installs a new dapptools test repository to use

TESTDATA=testdata
REPO=$1

cd $TESTDATA
git submodule add $REPO
