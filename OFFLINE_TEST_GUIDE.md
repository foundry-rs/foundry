# Offline Fork Testing Guide

Validate that Anvil's offline fork mode prevents all external RPC calls using Docker with network isolation.

## Prerequisites

- Docker & Docker Compose
- Saved state file (see below)

## Step 1: Create State File

```bash
# Build and run anvil with fork
cargo build --release -p anvil
./target/release/anvil \
  --fork-url https://sepolia.base.org \
  --optimism \
  --fork-block-number 20702367 \
  --fork-chain-id 84532 \
  --dump-state state.json
# Press Ctrl+C when done

# Or save via RPC while running:
cast rpc anvil_dumpState > state.json
```

## Step 2: Test Offline Mode

```bash
# Build and run with blocked internet (first build takes 10-20 min)
docker-compose -f docker-compose.offline-test.yml up --build anvil-offline
```

This blocks all outbound internet using iptables. Test it works:

```bash
cast block-number --rpc-url http://localhost:8545
cast balance 0xYourAddress --rpc-url http://localhost:8545
```

## Verification Methods

**1. Invalid URL**: Change `fork-url` in docker-compose to `https://invalid.test` - if anvil still works, it's offline

**2. Check connections**:
```bash
CONTAINER_ID=$(docker ps | grep anvil-offline | awk '{print $1}')
docker exec $CONTAINER_ID ss -tunp  # Should show only listening socket
```

**3. Query missing data**:
```bash
# Address NOT in state.json returns 0 instantly (no RPC call)
cast balance 0x0000000000000000000000000000000000000042 --rpc-url http://localhost:8545
```

## Customization

Edit `docker-compose.offline-test.yml` command section for your chain.

**Fund accounts** (optional):
```yaml
--fund-accounts 0xAddress1:1000 0xAddress2:5000  # Amounts in ETH
```

## Troubleshooting

- **"No such file: state.json"** - Ensure file exists in foundry directory
- **"failed to create offline provider"** - Expected in offline mode, anvil continues normally
- **First build slow (10-20 min)** - Normal, subsequent builds are cached
- **Container exits** - Check logs: `docker-compose -f docker-compose.offline-test.yml logs`

## Cleanup

```bash
docker-compose -f docker-compose.offline-test.yml down
docker rmi foundry-anvil-offline
```
