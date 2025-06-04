# Polkadot Foundry Release Test Suite

This folder contains scripts and resources for running comprehensive release tests against the [foundry-polkadot](https://github.com/paritytech/foundry-polkadot) toolchain release in a reproducible Docker environment. It is designed to verify the functionality of `forge` and `cast` commands, and to help maintainers validate new releases or changes.

## Limitations of the Test Suite

- **Return Code Only:**
  - The scripts in this suite only check that each command returns a zero (0) exit code, indicating success at the process level.
  - They do **not** inspect or validate the actual content of the command outputs.
  - As a result, a successful run (all commands return 0) does **not** guarantee that all features or commands are working 100% as expected—only that they did not fail fatally.
  - For full verification, manual inspection of the output logs (`forge.txt`, `cast.txt`) or more advanced automated output validation would be required.
- **RPC Endpoint Dependency:**
  - The reliability and results of the test suite depend on the behavior of the configured RPC URL (e.g., Paseo, Westend, etc.).
  - If the RPC endpoint is unstable, down, or changes its behavior, tests may fail or produce inconsistent results, even if the toolchain itself is functioning correctly.
  - **Note**: The test suite is currently configured to use the [Paseo](https://testnet-passet-hub-eth-rpc.polkadot.io) RPC endpoint.

## Contents

- **Dockerfile**: Builds a Docker image with the required `foundry-polkadot` binaries. The `version` is configurable as parameter.
- **forge.sh**: Script to run a suite of `forge` commands and capture their output.
- **cast.sh**: Script to run a suite of `cast` commands and capture their output.
- **forge.txt**: Output log from the last run of `forge.sh`.
- **cast.txt**: Output log from the last run of `cast.sh`.
- **test/**: Directory for storing test artifacts.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) installed and running.
- [Polkadot Foundry](https://github.com/paritytech/foundry-polkadot/releases/) release available.

## How to Run

### 1. Build the Docker Image

You can specify the Foundry version using the `VERSION` build argument. If omitted, it defaults to `stable`.

```sh
cd release-test
docker build --build-arg VERSION=1.1.0-rc3 -t foundry .
```

Or, to use the default `stable` version:

```sh
cd release-test
docker build -t foundry .
```

Alternatively, you can get the latest changes from the `master` branch to use the latest version of the `foundry-polkadot` repository:

Comment out the `ENTRYPOINT` line in the `Dockerfile` to use the default entrypoint (and not `ENTRYPOINT ["/bin/sh", "-c"]`).

```sh
docker build --platform=linux/amd64 -t foundry .
```

Update `forge.sh` and `cast.sh` under `docker_run()` to use `docker run --platform=linux/amd64` instead of `docker run` to run the commands.

### 2. Run the Test Scripts

#### Logical Prerequisites for Test Scripts

- **Forge Test Suite:**
  - Requires a wallet to be created in advance, with the address and private key specified in the script (see `ADDRESS` and `PRIVATE_KEY` variables in `forge.sh`).
  - The wallet should have sufficient testnet [funds](https://faucet.polkadot.io/?parachain=1111) if you are broadcasting transactions. Currently, the RPC URL is set to the [Passet Hub](https://testnet-passet-hub-eth-rpc.polkadot.io).

- **Cast Test Suite:**
  - Requires a previously deployed contract, with its address specified in the script (see `CONTRACT_ADDRESS` in `cast.sh`).
  - Also requires a wallet (address and private key) as above.
  - The contract should be deployed on the same network as specified by `RPC_URL`.

#### Forge Test Suite

```sh
# chmod +x forge.sh
./forge.sh
```

- This will:
  - Build a clean test environment in a Docker container.
  - Run a series of `forge` commands (`init`, `build`, `deploy`, etc.).
  - Log all output to `forge.txt`.
  - Stop on the first error.

#### Cast Test Suite

```sh
# chmod +x cast.sh
./cast.sh
```

- This will:
  - Build a clean test environment in a Docker container.
  - Run a series of `cast` commands (`wallet`, `call`, `decode`, etc.).
  - Log all output to `cast.txt`.
  - Stop on the first error.

### 3. Review Results

- Check `forge.txt` and `cast.txt` for command outputs and any errors.
- These files are overwritten on each run.

## Modifying the Release Test

- **To add or change tested commands:**  
  Edit `forge.sh` or `cast.sh` and add / remove / modify the `docker_run` lines as needed.  
  Each command is run inside the Docker container and its output is appended to the respective `.txt` file.

- **To change the foundry-polkadot version:**  
  Pass a different `VERSION` when building the Docker image (see above).  
  Rebuild the Docker image after making changes:

  ```sh
  docker build --build-arg VERSION=1.1.0-rc4 -t foundry .
  ```

- **To change the RPC endpoint or test accounts:**  
  Edit the `RPC_URL`, `ADDRESS`, or `PRIVATE_KEY` variables at the top of the scripts.

## How to Run Interactively

If you want to run a command interactively, you can use the following command to get a shell in the container:

```sh
docker run --rm -it foundry sh
```

For example, to run `forge --help` in the container while mounted the current `test/` directory:

```sh
docker run --rm -v $PWD/test:/test -w /test foundry forge --help
```

## Notes

- If a script fails, check the output log (`forge.txt` or `cast.txt`) for the last successful command and the error message.
- These scripts are intended for CI and release validation, but can also be run locally for manual testing.
- The test suite is destructive to the `test/` directory it creates inside the container; do not use for persistent data.
