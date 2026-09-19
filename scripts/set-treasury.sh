#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="${ENV_FILE:-$ROOT/deploy/devnet.env}"
NEW_TREASURY="${1:-}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE"
  exit 1
fi
# shellcheck disable=SC1090
source "$ENV_FILE"

if [[ -z "${SC:-}" && -z "${2:-}" ]]; then
  echo "Usage: SC=erd1qqqq... ./scripts/set-treasury.sh <erd1 treasury>"
  echo "   or: ./scripts/set-treasury.sh <erd1 treasury> <erd1 contract>"
  exit 1
fi

ADDR="${NEW_TREASURY}"
CONTRACT="${2:-${SC:-}}"

if [[ ! "$ADDR" =~ ^erd1 ]]; then
  echo "Treasury must be an erd1 address"
  exit 1
fi

echo "setTreasury → $ADDR on $CONTRACT"
mxpy contract call "$CONTRACT" \
  --function setTreasury \
  --arguments "$ADDR" \
  --pem "$PEM" \
  --proxy "$PROXY" \
  --chain "$CHAIN" \
  --recall-nonce \
  --gas-limit 5000000 \
  --send

echo "Verify: mxpy contract query $CONTRACT --function getTreasury --proxy $PROXY"
