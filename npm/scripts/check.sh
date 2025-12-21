#!/usr/bin/env bash

set -eou pipefail

# Ensure we're in the npm directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NPM_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$NPM_DIR" || exit 1

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

tools=(cast anvil forge chisel)

# Detect current platform
if [[ "$(uname)" == "Darwin" ]]; then
  if [[ "$(uname -m)" == "arm64" ]]; then
    TARGET="aarch64-apple-darwin"
    PLATFORM="darwin"
    ARCH="arm64"
  else
    TARGET="x86_64-apple-darwin"
    PLATFORM="darwin"
    ARCH="amd64"
  fi
elif [[ "$(uname)" == "Linux" ]]; then
  if [[ "$(uname -m)" == "aarch64" ]]; then
    TARGET="aarch64-unknown-linux-gnu"
    PLATFORM="linux"
    ARCH="arm64"
  else
    TARGET="x86_64-unknown-linux-gnu"
    PLATFORM="linux"
    ARCH="amd64"
  fi
else
  echo -e "${RED}Unsupported platform: $(uname)${NC}"
  exit 1
fi

# Check if we're in CI and can use artifacts
USE_ARTIFACTS=false
if [[ -n "${CI:-}" ]] && [[ -n "${ARTIFACT_DIR:-}" ]] && [[ -n "${RELEASE_VERSION:-}" ]]; then
  USE_ARTIFACTS=true
  echo -e "${BLUE}=== Foundry npm Package Check (CI Mode) ===${NC}"
  echo -e "${BLUE}Using artifacts from: ${ARTIFACT_DIR}${NC}"
  echo -e "${BLUE}Release version: ${RELEASE_VERSION}${NC}"
else
  echo -e "${BLUE}=== Foundry npm Package Check (Local Mode) ===${NC}"
  echo -e "${BLUE}Platform: ${PLATFORM}${NC}"
  echo -e "${BLUE}Architecture: ${ARCH}${NC}"
  echo -e "${BLUE}Target: ${TARGET}${NC}"
fi
echo ""

# Determine total step count and current step
TOTAL_STEPS=$([[ "$USE_ARTIFACTS" == "true" ]] && echo "5" || echo "6")

# Step 1: Build binaries or stage from artifacts
if [[ "$USE_ARTIFACTS" == "true" ]]; then
  echo -e "${YELLOW}[1/${TOTAL_STEPS}] Staging packages from artifacts…${NC}"
  for tool in "${tools[@]}"; do
    echo -e "  Staging ${tool} from artifacts…"
    bun ./scripts/stage-from-artifact.mjs \
      --tool "$tool" \
      --platform "$PLATFORM" \
      --arch "$ARCH" \
      --release-version "$RELEASE_VERSION" \
      --artifact-dir "$ARTIFACT_DIR" || {
      echo -e "${RED}Failed to stage ${tool} from artifacts${NC}"
      exit 1
    }
  done
  echo -e "${GREEN}✓ All packages staged from artifacts${NC}"
else
  echo -e "${YELLOW}[1/${TOTAL_STEPS}] Building tools…${NC}"
  for tool in "${tools[@]}"; do
    echo -e "  Building ${tool}…"
    cargo build \
      --package "$tool" \
      --target "$TARGET" \
      --release || {
      echo -e "${RED}Failed to build ${tool}${NC}"
      exit 1
    }
  done
  echo -e "${GREEN}✓ All tools built successfully${NC}"
fi
echo ""

# Step 2: Stage platform-specific packages (if not using artifacts)
if [[ "$USE_ARTIFACTS" == "false" ]]; then
  echo -e "${YELLOW}[2/${TOTAL_STEPS}] Staging platform-specific packages…${NC}"
  for tool in "${tools[@]}"; do
    echo -e "  Staging ${tool}…"
    BIN_PATH="../target/${TARGET}/release/${tool}"
    if [[ "$PLATFORM" == "win32" ]]; then
      BIN_PATH="../target/${TARGET}/release/${tool}.exe"
    fi
    
    if [[ ! -f "$BIN_PATH" ]]; then
      echo -e "${RED}Binary not found: ${BIN_PATH}${NC}"
      exit 1
    fi
    
    PLATFORM_NAME="$PLATFORM" ARCH="$ARCH" bun ./scripts/prepublish.mjs \
      --tool "$tool" --bin-path "$BIN_PATH" || {
      echo -e "${RED}Failed to stage ${tool}${NC}"
      exit 1
    }
  done
  echo -e "${GREEN}✓ All platform-specific packages staged${NC}"
  echo ""
fi

# Step 2/3: Verify platform-specific packages
STEP_NUM=$([[ "$USE_ARTIFACTS" == "true" ]] && echo "2" || echo "3")
echo -e "${YELLOW}[${STEP_NUM}/${TOTAL_STEPS}] Verifying platform-specific packages…${NC}"
for tool in "${tools[@]}"; do
  PACKAGE_DIR="@foundry-rs/${tool}-${PLATFORM}-${ARCH}"
  BIN_NAME="${tool}"
  if [[ "$PLATFORM" == "win32" ]]; then
    BIN_NAME="${tool}.exe"
  fi
  
  PACKAGE_JSON="${PACKAGE_DIR}/package.json"
  BIN_FILE="${PACKAGE_DIR}/bin/${BIN_NAME}"
  
  if [[ ! -f "$PACKAGE_JSON" ]]; then
    echo -e "${RED}Missing package.json: ${PACKAGE_JSON}${NC}"
    exit 1
  fi
  
  if [[ ! -f "$BIN_FILE" ]]; then
    echo -e "${RED}Missing binary: ${BIN_FILE}${NC}"
    exit 1
  fi
  
  if [[ "$PLATFORM" != "win32" ]] && [[ ! -x "$BIN_FILE" ]]; then
    echo -e "${RED}Binary not executable: ${BIN_FILE}${NC}"
    exit 1
  fi
  
  # Validate package.json structure
  if ! bun -e "
    const pkg = JSON.parse(await Bun.file('${PACKAGE_JSON}').text());
    if (!pkg.name || !pkg.bin || !pkg.bin['${tool}']) {
      console.error('Invalid package.json structure');
      process.exit(1);
    }
  "; then
    echo -e "${RED}Invalid package.json: ${PACKAGE_JSON}${NC}"
    exit 1
  fi
  
  echo -e "  ✓ ${tool}-${PLATFORM}-${ARCH}"
done
echo -e "${GREEN}✓ All platform-specific packages verified${NC}"
echo ""

# Step 3/4: Prepare meta packages
STEP_NUM=$([[ "$USE_ARTIFACTS" == "true" ]] && echo "3" || echo "4")
echo -e "${YELLOW}[${STEP_NUM}/${TOTAL_STEPS}] Preparing meta packages…${NC}"
# Use RELEASE_VERSION if available, otherwise a dummy version for testing
TEST_VERSION="${RELEASE_VERSION:-0.0.0-test}"
for tool in "${tools[@]}"; do
  echo -e "  Preparing ${tool} meta package…"
  RELEASE_VERSION="$TEST_VERSION" bun ./scripts/publish-meta.mjs --tool "$tool" --release-version "$TEST_VERSION" || {
    echo -e "${RED}Failed to prepare ${tool} meta package${NC}"
    exit 1
  }
done
echo -e "${GREEN}✓ All meta packages prepared${NC}"
echo ""

# Step 4/5: Verify meta packages
STEP_NUM=$([[ "$USE_ARTIFACTS" == "true" ]] && echo "4" || echo "5")
echo -e "${YELLOW}[${STEP_NUM}/${TOTAL_STEPS}] Verifying meta packages…${NC}"
for tool in "${tools[@]}"; do
  META_DIR="@foundry-rs/${tool}"
  PACKAGE_JSON="${META_DIR}/package.json"
  BIN_MJS="${META_DIR}/bin.mjs"
  CONST_MJS="${META_DIR}/const.mjs"
  POSTINSTALL_MJS="${META_DIR}/postinstall.mjs"
  
  if [[ ! -f "$PACKAGE_JSON" ]]; then
    echo -e "${RED}Missing package.json: ${PACKAGE_JSON}${NC}"
    exit 1
  fi
  
  if [[ ! -f "$BIN_MJS" ]]; then
    echo -e "${RED}Missing bin.mjs: ${BIN_MJS}${NC}"
    exit 1
  fi
  
  if [[ ! -f "$CONST_MJS" ]]; then
    echo -e "${RED}Missing const.mjs: ${CONST_MJS}${NC}"
    exit 1
  fi
  
  if [[ ! -f "$POSTINSTALL_MJS" ]]; then
    echo -e "${RED}Missing postinstall.mjs: ${POSTINSTALL_MJS}${NC}"
    exit 1
  fi
  
  # Verify import map points to const.mjs
  if ! bun -e "
    const pkg = JSON.parse(await Bun.file('${PACKAGE_JSON}').text());
    if (pkg.imports?.['#const.mjs'] !== './const.mjs') {
      console.error('Invalid import map: #const.mjs should point to ./const.mjs');
      console.error('Found:', pkg.imports?.['#const.mjs']);
      process.exit(1);
    }
    if (pkg.bin?.['${tool}'] !== './bin.mjs') {
      console.error('Invalid bin entry');
      process.exit(1);
    }
  "; then
    echo -e "${RED}Invalid package.json structure for ${tool}${NC}"
    exit 1
  fi
  
  # Verify bin.mjs can import from const.mjs
  CONST_ABS_PATH="$(cd "$META_DIR" && pwd)/const.mjs"
  if ! bun -e "
    try {
      // Try to import const.mjs to verify it exports correctly
      const constModule = await import('${CONST_ABS_PATH}');
      const required = ['BINARY_NAME', 'colors', 'KNOWN_TOOLS', 'PLATFORM_SPECIFIC_PACKAGE_NAME', 'resolveTargetTool'];
      for (const name of required) {
        if (!(name in constModule)) {
          console.error(\`Missing export: \${name}\`);
          process.exit(1);
        }
      }
    } catch (error) {
      console.error('Failed to import const.mjs:', error.message);
      process.exit(1);
    }
  "; then
    echo -e "${RED}Failed to validate const.mjs exports for ${tool}${NC}"
    exit 1
  fi
  
  echo -e "  ✓ ${tool}"
done
echo -e "${GREEN}✓ All meta packages verified${NC}"
echo ""

# Step 5/6: Test package packing
STEP_NUM=$([[ "$USE_ARTIFACTS" == "true" ]] && echo "5" || echo "6")
echo -e "${YELLOW}[${STEP_NUM}/${TOTAL_STEPS}] Testing package packing…${NC}"
for tool in "${tools[@]}"; do
  META_DIR="@foundry-rs/${tool}"
  echo -e "  Packing ${tool}…"
  
  # Test that we can pack the meta package
  ORIG_DIR=$(pwd)
  cd "$META_DIR" || exit 1
  
  # Try to pack and capture output
  PACK_OUTPUT=$(bun pm pack 2>&1)
  PACK_EXIT=$?
  
  # Clean up any generated .tgz files
  rm -f -- *.tgz
  
  cd "$ORIG_DIR" || exit 1
  
  if [[ $PACK_EXIT -ne 0 ]]; then
    echo -e "${RED}Failed to pack ${tool}${NC}"
    echo "$PACK_OUTPUT"
    exit 1
  fi
  
  echo -e "  ✓ ${tool}"
done
echo -e "${GREEN}✓ All packages can be packed${NC}"
echo ""

echo -e "${GREEN}=== All checks passed! ===${NC}"
echo -e "${BLUE}Ready to publish${NC}"