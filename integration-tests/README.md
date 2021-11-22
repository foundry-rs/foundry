# Benchmarks & Integration Tests

## Motivation

This sub-project is used for integration testing
[forge](https://github.com/gakonst/foundry/) with common dapptools repositories,
to ensure that it's compatible with the test cases in them, e.g. usage of HEVM
cheatcodes, proper forking mode integration, fuzzing etc.

It is also used for getting quick performance benchmarks for Forge.

## How to run?

1. Make sure forge & dapptools are installed
1. Run `./test.sh $REPO_NAME`, e.g. `./test.sh LootLoose`.

## Repositories Included

See the submodules linked within the [`testdata/`](./testdata) folder.

## Adding a new repository

We use git submodules (I know, I know submodules are not great, feel free to
recommend a working alternative), you can add a new one via:
`./add_test.sh $URL`
