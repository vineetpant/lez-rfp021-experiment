#!/usr/bin/env bash
# Bring the devnet up: bedrock, then the LEZ indexer and sequencer, all as native processes.
#
# Needs no logos-execution-zone checkout. Bedrock is built from its pinned commit and the
# LEZ services are cargo-installed from the pinned tag, both into .devnet/bin.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

mkdir -p "$LOGS" "$DEVNET/state/bedrock" "$DEVNET/state/indexer" "$LEE_WALLET_HOME_DIR"

step "Checking prerequisites"
command -v cargo >/dev/null || die "cargo not found - see README 'Prerequisites'"
echo "  cargo    ok"

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

step "Installing bedrock"
ensure_bedrock_binary

step "Installing the LEZ service binaries"
ensure_devnet_binaries

# Bedrock keeps its chain under its working directory, so stopping and starting the devnet
# resumes the same chain. POL_PROOF_DEV_MODE skips leader proofs, as LEZ's own devnet does.
step "Starting bedrock"
( cd "$DEVNET/state/bedrock" || exit 1
  POL_PROOF_DEV_MODE=true nohup "$DEVNET_BIN/logos-blockchain-node" \
    "$HELLO_DIR/configs/bedrock/node-config.yaml" \
    --deployment "$HELLO_DIR/configs/bedrock/deployment-settings.yaml" \
    < /dev/null > "$LOGS/bedrock.log" 2>&1 &
  echo "  pid $!" )
for i in $(seq 1 30); do
    curl -fsS -o /dev/null http://localhost:18080/cryptarchia/info 2>/dev/null && break
    [ "$i" -eq 30 ] && { tail -20 "$LOGS/bedrock.log" >&2; die "bedrock did not answer on 18080"; }
    sleep 1
done
echo "  answering on 18080"

# The wallet reads its config from its home directory.
cp -f "$HELLO_DIR/configs/wallet/wallet_config.json" "$LEE_WALLET_HOME_DIR/"

# Each service puts its RocksDB in the working directory, so each gets its own state dir.
# Run two from one directory and the second dies on the first's lock.
#
# Only the service itself goes to the background, with stdin closed. Backgrounding `cd && cmd`
# leaves a subshell alive holding this script's stdout, so anything reading the script's
# output waits for the services to exit.
step "Starting indexer"
( cd "$DEVNET/state/indexer" || exit 1
  RUST_LOG=info nohup env "${DEV_MODE_ENV[@]}" \
    "$DEVNET_BIN/indexer_service" "$HELLO_DIR/configs/indexer_config.json" \
    < /dev/null > "$LOGS/indexer.log" 2>&1 &
  echo "  pid $!" )

# `setup` writes the sequencer's home once: its config, from the network file and genesis,
# and a fresh channel signing key. Later runs start from that home and resume its chain.
step "Starting sequencer"
SEQ_HOME="$DEVNET/state/sequencer"
if [ ! -f "$SEQ_HOME/sequencer_config.json" ]; then
    "$DEVNET_BIN/sequencer_service" setup "$SEQ_HOME" \
        --config "$HELLO_DIR/configs/sequencer_network.json" \
        --funding-public-key "$BEDROCK_FUNDING_PUBLIC_KEY" --create-channel \
        > "$LOGS/sequencer-setup.log" 2>&1 \
        || { cat "$LOGS/sequencer-setup.log" >&2; die "sequencer setup failed"; }
    echo "  set up $SEQ_HOME"
fi
( cd "$SEQ_HOME" || exit 1
  RUST_LOG="info,risc0_zkvm=warn,kameo=warn" nohup env "${DEV_MODE_ENV[@]}" \
    "$DEVNET_BIN/sequencer_service" start "$SEQ_HOME" \
    < /dev/null > "$LOGS/sequencer.log" 2>&1 &
  echo "  pid $!" )

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
