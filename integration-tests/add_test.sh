#!/bin/bash
# Installs a new dapptools test repository to use

CURRENT_DIR=$(dirname "$0")
TESTDATA=testdata
REPO_URL=https://github.com/$1
REPO=${REPO_URL##*/}

git clone --depth 1 $REPO_URL $CURRENT_DIR/$TESTDATA/$REPO
