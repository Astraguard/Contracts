#!/usr/bin/env bash
set -euo pipefail

# Usage: scripts/deploy.sh <network> <source-account>
# Deploys escrow, insurance-pool, and registry-anchor and records the
# resulting contract IDs in deployments/<network>.json.
#
# Run scripts/build.sh first so the optimized wasm files exist. This script
# only deploys — it does NOT call initialize() on any contract, since each
# contract's constructor args (admin, oracle, asset, committee, ...) depend
# on the target network's addresses.

NETWORK="${1:?usage: deploy.sh <network> <source-account>}"
SOURCE="${2:?usage: deploy.sh <network> <source-account>}"

cd "$(dirname "$0")/.."
WASM_DIR="target/wasm32v1-none/release"

deploy_one() {
  local wasm_name="$1"
  stellar contract deploy \
    --wasm "${WASM_DIR}/${wasm_name}.optimized.wasm" \
    --source "${SOURCE}" \
    --network "${NETWORK}"
}

echo "Deploying escrow..."
ESCROW_ID=$(deploy_one escrow)

echo "Deploying insurance-pool..."
INSURANCE_ID=$(deploy_one insurance_pool)

echo "Deploying registry-anchor..."
REGISTRY_ID=$(deploy_one registry_anchor)

mkdir -p deployments
cat > "deployments/${NETWORK}.json" <<JSON
{
  "network": "${NETWORK}",
  "contracts": {
    "escrow": "${ESCROW_ID}",
    "insurance-pool": "${INSURANCE_ID}",
    "registry-anchor": "${REGISTRY_ID}"
  }
}
JSON

echo "Recorded deployment addresses in deployments/${NETWORK}.json"
echo "Remember to call each contract's initialize() before use."
