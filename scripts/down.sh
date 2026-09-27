#!/usr/bin/env bash
# Stop the devnet. Leaves state and installed binaries; reset.sh wipes the state.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "Stopping indexer and sequencer"
# Match the binary name wherever it was launched from, not only .devnet/bin. An earlier
# layout ran these out of a logos-execution-zone checkout, and a path-specific pattern left
# those alive holding the ports -- which then answered for a devnet that was not running.
# The bracket keeps pkill from matching its own command line.
pkill -f "[/]indexer_service" && echo "  indexer stopped" || echo "  indexer not running"
pkill -f "[/]sequencer_service" && echo "  sequencer stopped" || echo "  sequencer not running"

step "Stopping bedrock"
( cd "$HELLO_DIR" && docker compose down ) || true

echo
echo "Devnet stopped. State kept - run ./scripts/reset.sh to wipe it."
