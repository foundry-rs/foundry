# Benchmarks & Integration Tests

## Motivation

This sub-project is used for integration testing
[forge](https://github.com/gakonst/foundry/) with common dapptools repositories,
to ensure that it's compatible with the test cases in them, e.g. usage of HEVM
cheatcodes, proper forking mode integration, fuzzing etc.

It is also used for getting quick performance benchmarks for Forge.

## How to run?

1. Make sure forge & dapptools are installed
2. Clone testdata with `make testdata` from the Foundry project root
3. Run `./test.sh $REPO_NAME`, e.g. `./test.sh LootLoose`

## Repositories Included

See the repositories listed in [Makefile](../Makefile)

## Adding a new repository

Previously we used git submodules, but it's not great because `cargo`
[doesn't have](https://github.com/rust-lang/cargo/issues/4247) an option to ignore submodules when installing.

Now we use simple `git clone --depth 1 --recursive` inside `Makefile`.

To add new repository, see `INTEGRATION_TESTS_REPOS` variable in [Makefile](../Makefile) 
