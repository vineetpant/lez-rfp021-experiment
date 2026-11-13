#!/usr/bin/env bash
# Stop the devnet, keeping its state and installed binaries. With --reset, also wipe the chain
# and wallet state, so the next run starts from genesis.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"

step "Stopping indexer and sequencer"
# Match the binary name wherever it was launched from, not only .devnet/bin. An earlier
# layout ran these out of a logos-execution-zone checkout, and a path-specific pattern left
# those alive holding the ports -- which then answered for a devnet that was not running.
# The bracket keeps pkill from matching its own command line.
pkill -f "[/]indexer_service" && echo "  indexer stopped" || echo "  indexer not running"
pkill -f "[/]sequencer_service" && echo "  sequencer stopped" || echo "  sequencer not running"

step "Stopping bedrock"
pkill -f "[/]logos-blockchain-node" && echo "  bedrock stopped" || echo "  bedrock not running"

if [ "${1:-}" = --reset ]; then
    step "Removing chain and wallet state"
    rm -rf "$DEVNET/state" "$LOGS"
    # The wallet's own store too: without this, account labels collide on the next run.
    rm -f "$LEE_WALLET_HOME_DIR/storage.json" "$LEE_WALLET_HOME_DIR/statistics.json"
    echo "  removed .devnet/state, .devnet/logs and the wallet store"
    echo
    echo "Clean. Next: ./devnet/up.sh"
else
    echo
    echo "Devnet stopped. State kept - run ./devnet/down.sh --reset to wipe it."
fi
