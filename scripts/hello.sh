#!/usr/bin/env bash
# Build this project, deploy the guest program to the local devnet, call it, and check
# the greeting round-trips. Run ./scripts/up.sh first.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$HELLO_DIR"

[ -x "$DEVNET_BIN/wallet" ] || die "no wallet in .devnet/bin - run ./scripts/up.sh first"

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
    RISC0_DOCKER_CONTAINER_TAG="$GUEST_BUILDER_TAG" \
        cargo risczero build --manifest-path guest/Cargo.toml
    GUEST_BIN=$(find guest/target -name "hello_world.bin" -type f | head -1)
    [ -n "$GUEST_BIN" ] || die "guest build produced no hello_world.bin"
fi
echo "  $GUEST_BIN"

step "Importing the funded genesis account as fee payer"
wallet account import public --private-key "$PAYER_KEY" 2>&1 \
    | grep -E "Imported|already" | sed 's/^/  /' || echo "  already imported"

step "Creating a target account for the greeting"
TARGET=$(wallet account new public --label "hello-$(date +%s)" \
    | grep -oE "Public/[1-9A-HJ-NP-Za-km-z]+" | head -1)
[ -n "$TARGET" ] || die "could not create a target account"
echo "  $TARGET"

step "Deploying and calling the program"
OUT=$(printf '%s\n' "$WALLET_PASSWORD" | env "${DEV_MODE_ENV[@]}" \
        ./target/release/hello_world "$GUEST_BIN" "${TARGET#Public/}" "$PAYER_ID" 2>&1) \
    || { echo "$OUT" | tail -20; die "the runner failed"; }
echo "$OUT" | grep -E "deployed|submitted|included in block" | sed 's/^/  /'

PROGRAM=$(echo "$OUT" | grep -oE "header account [1-9A-HJ-NP-Za-km-z]+" | awk '{print $3}' | head -1)
[ -n "$PROGRAM" ] || { echo "$OUT" | tail -20; die "could not parse the deployed program id"; }

step "Reading the greeting back out of the account shard"
# `--scope all` reports an uninitialised account even when the shard holds data, so the
# read names the program explicitly.
#
# The read polls: submission only means the transaction was accepted into the mempool, and
# the shard is not readable until the block carrying it has been executed.
HEX=""
for i in $(seq 1 20); do
    READ=$(wallet account get --raw --scope "$PROGRAM" --account-id "$TARGET" 2>&1) || true
    # The shard value is the hex string; the account id key is base58 and never matches.
    HEX=$(grep -oE '"[0-9a-f]{6,}"' <<<"$READ" | tr -d '"' | head -1 || true)
    [ -n "$HEX" ] && break
    sleep 3
done

if [ -z "$HEX" ]; then
    echo "  last read returned:" >&2
    sed 's/^/    /' <<<"$READ" >&2
    die "no shard data for $TARGET under program $PROGRAM after 60s"
fi

DECODED=$(echo "$HEX" | xxd -r -p)
echo "  shard hex: $HEX"
echo "  decoded:   $DECODED"

if [ "$DECODED" = "Hola mundo!" ]; then
    printf '\n\033[1;32mPASS\033[0m  the program ran on LEZ and the greeting round-tripped.\n'
else
    printf '\n\033[1;31mFAIL\033[0m  expected "Hola mundo!", got "%s"\n' "$DECODED"
    exit 1
fi
