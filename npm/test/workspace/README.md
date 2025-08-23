# Test package deployment

## Run local package registry server

```bash
docker run -it --rm --name verdaccio -p 4873:4873 verdaccio/verdaccio
```

## Clean up previous test

```bash
/bin/bash ./scripts/setup.sh
```

## publish to local registry

```bash
# pwd should be foundry/npm
/bin/bash ./scripts/local-setup.sh
```

Then, `cd` back to `foundry/npm/test/workspace`, then:

## Install forge

```bash
bun add @foundry-rs/forge --no-cache --force
```

## Run forge

```bash
bun x forge --version
```
