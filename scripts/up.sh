#!/usr/bin/env bash
# Bring the devnet up: bedrock in Docker, then the LEZ indexer and sequencer.
#
# Needs no logos-execution-zone checkout. Bedrock's image pulls a published release
# binary; the LEZ services are cargo-installed from the pinned tag into .devnet/bin.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

mkdir -p "$LOGS" "$DEVNET/state/indexer" "$DEVNET/state/sequencer" "$LEE_WALLET_HOME_DIR"

step "Checking prerequisites"
for c in cargo docker; do
    command -v "$c" >/dev/null || die "$c not found - see README 'Prerequisites'"
    printf '  %-8s ok\n' "$c"
done
docker info >/dev/null 2>&1 || die "Docker is not running"
echo "  docker   running"

# Refuse to run against someone else's devnet. Without this check a leftover sequencer --
# from an earlier layout, or another checkout -- keeps the port and answers every health
# check, so the devnet looks up while the services just started are dead.
step "Checking the ports are free"
for spec in "18080 bedrock" "3040 sequencer" "8779 indexer"; do
    port=${spec%% *}; what=${spec##* }
    if holder=$(lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null | head -1) && [ -n "$holder" ]; then
        echo "  port $port ($what) is held by pid $holder:" >&2
        ps -o command= -p "$holder" | sed 's/^/    /' >&2
        die "run ./scripts/down.sh first, or stop that process"
    fi
    printf '  %-5s free (%s)\n' "$port" "$what"
done

step "Installing the LEZ service binaries"
ensure_devnet_binaries

step "Starting bedrock (first run builds the image: ~1 min)"
( cd "$HELLO_DIR" && docker compose up -d --build )

# The wallet reads its config from its home directory.
cp -f "$HELLO_DIR/configs/wallet/wallet_config.json" "$LEE_WALLET_HOME_DIR/"

# Each service puts its RocksDB in the working directory, so each gets its own state dir.
# Run two from one directory and the second dies on the first's lock.
step "Starting indexer"
( cd "$DEVNET/state/indexer" && RUST_LOG=info env "${DEV_MODE_ENV[@]}" \
    nohup "$DEVNET_BIN/indexer_service" "$HELLO_DIR/configs/indexer_config.json" \
    > "$LOGS/indexer.log" 2>&1 & echo "  pid $!" )

step "Starting sequencer"
( cd "$DEVNET/state/sequencer" && RUST_LOG=info env "${DEV_MODE_ENV[@]}" \
    nohup "$DEVNET_BIN/sequencer_service" "$HELLO_DIR/configs/sequencer_config.json" \
    > "$LOGS/sequencer.log" 2>&1 & echo "  pid $!" )

step "Waiting for the chain to accept the wallet"
# A check right after start can return "Failed to find leader" while the sequencer
# registers, so this retries. The first call also performs wallet setup, which prints a
# recovery phrase.
for i in $(seq 1 30); do
    health=$(wallet check-health 2>&1) || true
    if grep -q "All looks good" <<<"$health"; then
        printf '  healthy after %ds\n\n' $((i * 10))
        echo "Devnet up. Logs in $LOGS"
        echo "Next: ./scripts/hello.sh"
        exit 0
    fi
    printf '%s\n--- attempt %d ---\n%s\n' "$(date -u +%H:%M:%S)" "$i" "$health" >> "$LOGS/health.log"
    sleep 10
done

echo >&2
echo "Sequencer did not become healthy within 5 minutes." >&2
echo "--- last check-health output ---" >&2
tail -20 <<<"$health" >&2
echo "--- sequencer.log ---" >&2
tail -20 "$LOGS/sequencer.log" >&2
echo "(all attempts in $LOGS/health.log)" >&2
exit 1
