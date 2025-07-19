# Test package deployment

## Run local package registry server

```bash
docker run -it --rm --name verdaccio -p 4873:4873 verdaccio/verdaccio
```

## Clean up previous test

```bash
/bin/bash ./scripts/setup.sh
```

## Install forge

```bash
bun add @foundry-rs/forge --no-cache --force
```

## Run forge

```bash
bun x forge --version
```
