#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="${1:-$ROOT/deploy/devnet.env}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE"
  echo "cp deploy/devnet.env.example deploy/devnet.env  # then fill PEM + TREASURY + ROAR_TOKEN"
  exit 1
fi

# shellcheck disable=SC1090
source "$ENV_FILE"

WASM="$ROOT/output/pridevault-casino.wasm"
if [[ ! -f "$WASM" ]]; then
  echo "Build first:"
  echo "  cargo install multiversx-sc-meta --locked"
  echo "  sc-meta all build"
  exit 1
fi

if [[ ! -f "$PEM" ]]; then
  echo "PEM not found: $PEM"
  exit 1
fi

echo "Deploying PrideVault Casino to $CHAIN via $PROXY"

mxpy contract deploy \
  --bytecode "$WASM" \
  --proxy "$PROXY" \
  --chain "$CHAIN" \
  --pem "$PEM" \
  --recall-nonce \
  --gas-limit 80000000 \
  --send \
  --arguments \
    "$TREASURY" \
    "str:${ROAR_TOKEN}" \
    "$MIN_BET_EGLD" \
    "$CAP_MAX_EGLD" \
    "$MIN_BET_ROAR" \
    "$CAP_MAX_ROAR" \
    "$ROUND_BLOCKS" \
    "$MIN_BANKROLL_EGLD" \
    "$MIN_BANKROLL_ROAR"

echo
echo "Copy the contract address into PrideVault src/lib/sections.ts → GAMES.roarDice"
echo "Then seed bankroll:"
echo "  mxpy contract call <SC> --function fundBankroll --value $SEED_EGLD --pem $PEM --proxy $PROXY --chain $CHAIN --recall-nonce --gas-limit 5000000 --send"
echo "  mxpy contract call <SC> --function fundBankroll --token-transfers ${ROAR_TOKEN} ${SEED_ROAR} --pem $PEM --proxy $PROXY --chain $CHAIN --recall-nonce --gas-limit 8000000 --send"
