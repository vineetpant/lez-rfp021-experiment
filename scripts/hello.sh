#!/usr/bin/env bash
# Build this project, deploy the guest program to the local devnet, call it publicly and
# privately, and check the greeting round-trips both ways. Run ./scripts/up.sh first.
#
# Every charge the sequencer makes is printed as a FEE line and kept in .devnet/logs/fees.log.
# DEV_MODE=0 (set for up.sh too) proves for real; the default fakes every proof.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$HELLO_DIR"

[ -x "$DEVNET_BIN/wallet" ] || die "no wallet in .devnet/bin - run ./scripts/up.sh first"
FEES="$LOGS/fees.log"

step "Building the host binary"
cargo build --release

step "Compiling the guest program to RISC-V"
# The guest is compiled inside a fixed container: a program's id is the hash of its ELF,
# so the build has to be reproducible. GUEST_BUILDER_TAG overrides the default image,
# whose rustc is too old for this dependency tree.
#
# The container runs `cargo fetch --locked`, so the guest needs its own lockfile.
[ -f guest/Cargo.lock ] || cargo generate-lockfile --manifest-path guest/Cargo.toml

GUEST_BIN=$(find guest/target -name "hello_world.bin" -type f 2>/dev/null | head -1 || true)
if [ -n "$GUEST_BIN" ]; then
    echo "  already built: $GUEST_BIN"
else
    docker info >/dev/null 2>&1 || die "Docker is not running; the guest builds in a container"
    RISC0_DOCKER_CONTAINER_TAG="$GUEST_BUILDER_TAG" \
        cargo risczero build --manifest-path guest/Cargo.toml
    GUEST_BIN=$(find guest/target -name "hello_world.bin" -type f | head -1)
    [ -n "$GUEST_BIN" ] || die "guest build produced no hello_world.bin"
fi
echo "  $GUEST_BIN"

step "Importing the funded genesis account as fee payer"
PAYER=$(wallet account import public --private-key "$PAYER_KEY" 2>&1 \
    | grep -oE "Public/[1-9A-HJ-NP-Za-km-z]+" | head -1 || true)
if [ -z "$PAYER" ]; then
    PAYER=$(wallet account list)
    PAYER=$(grep -oE "Public/[1-9A-HJ-NP-Za-km-z]+" <<<"$PAYER" | head -1 || true)
fi
[ -n "$PAYER" ] || die "could not import the genesis payer"
echo "  $PAYER"

# Prints the account id a `wallet account new` call creates, found by its label.
new_account() {
    local kind=$1 label=$2 id
    wallet account new "$kind" --label "$label" >/dev/null
    id=$(wallet account list)
    id=$(grep -F "[$label]" <<<"$id" | grep -oE "(Public|Private)/[1-9A-HJ-NP-Za-km-z]+" | head -1 || true)
    [ -n "$id" ] || die "could not create $kind account $label"
    echo "$id"
}

# Runs the client, prints its result lines and appends its FEE lines to the fee log.
run_client() {
    local out
    out=$(env "${DEV_MODE_ENV[@]}" ./target/release/hello_world "$@" 2>&1) \
        || { echo "$out" | grep -v WARNING | tail -20; die "the runner failed"; }
    echo "$out" | grep -E "^(deployed|submitted|included|FEE)" | sed 's/^/  /'
    # Not every command charges anything; grep finding nothing must not end the script.
    { echo "$out" | grep -E "^FEE" | sed "s/^/$(date -u +%FT%TZ) dev_mode=$DEV_MODE /" >> "$FEES"; } || true
    echo "$out" | grep -oE "header account [1-9A-HJ-NP-Za-km-z]+" | awk '{print $3}' | head -1 >"$DEVNET/last-program"
}

# Reads the greeting back out of the account's actor state under the program. Polls: a
# private account's state reaches the wallet only once it has synced the block.
check_greeting() {
    local account=$1 program=$2 read hex
    for _ in $(seq 1 20); do
        [[ $account == Private/* ]] && wallet account sync-private >/dev/null 2>&1 || true
        read=$(wallet account get --raw --scope "$program" --account-id "$account" 2>&1) || true
        # The actor state is the hex string; the program id key is base58 and never matches.
        hex=$(grep -oE '"[0-9a-f]{6,}"' <<<"$read" | tr -d '"' | head -1 || true)
        [ -n "$hex" ] && break
        sleep 3
    done
    [ -n "$hex" ] || { sed 's/^/    /' <<<"$read" >&2; die "no greeting in $account under $program"; }
    local decoded
    decoded=$(echo "$hex" | xxd -r -p)
    echo "  actor state: $hex = \"$decoded\""
    [ "$decoded" = "Hola mundo!" ] || die "expected \"Hola mundo!\", got \"$decoded\""
}

step "Public call: deploy, call, read back"
TARGET=$(new_account public "hello-$(date +%s)")
run_client "$GUEST_BIN" "${TARGET#Public/}" "${PAYER#Public/}"
check_greeting "$TARGET" "$(cat "$DEVNET/last-program")"

step "Funding a private fee payer"
# A private transaction pays its fee inside the proof, from an owned private account, so one
# has to hold tokens first. Shielding them from the public payer is itself a private
# transaction.
FEE_PAYER=$(new_account private "fee-payer-$(date +%s)")
env "${DEV_MODE_ENV[@]}" "$DEVNET_BIN/wallet" auth-transfer send \
    --from "$PAYER" --to "$FEE_PAYER" --amount 100000000000 2>&1 \
    | grep -E "included|rror" | sed 's/^/  /'
echo "  $FEE_PAYER"

step "Private call: deploy, prove locally, submit the proof, read back"
TARGET=$(new_account private "hello-private-$(date +%s)")
wallet account sync-private >/dev/null
run_client "$GUEST_BIN" "${TARGET#Private/}" "${PAYER#Public/}" private
check_greeting "$TARGET" "$(cat "$DEVNET/last-program")"

printf '\n\033[1;32mPASS\033[0m  the program ran on LEZ publicly and privately, and the greeting round-tripped both ways.\n'
echo "Fees charged: $FEES"
