#!/usr/bin/env bash
# Wipe chain and wallet state so the next run starts from genesis. Keeps the installed
# binaries, the cargo target dirs and the compiled guest.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

"$(dirname "${BASH_SOURCE[0]}")/down.sh"

step "Removing chain and wallet state"
rm -rf "$DEVNET/state" "$LOGS"
# The wallet's own store too: without this, account labels collide on the next run.
rm -f "$LEE_WALLET_HOME_DIR/storage.json" "$LEE_WALLET_HOME_DIR/statistics.json"
echo "  removed .devnet/state, .devnet/logs and the wallet store"

echo
echo "Clean. Next: ./scripts/up.sh"
