#!/usr/bin/env bash
# The light client end to end on the local devnet, on recorded mainnet data (tests/data):
#
#   deploy eth_lc -> Init from the recorded checkpoint -> private updates, through the
#   committee handoffs to a finality update -> public attestation -> private attestation
#   -> negative attestations, which must fail
#
# Run ./devnet/up.sh first. Every charge is a FEE line, every proven step a TIME line, and
# every transaction's outcome a STATUS line, all kept in .devnet/logs/fees.log. DEV_MODE=0, for
# up.sh too, proves for real. After each private step, PUBLISHED lines list what its transaction
# made public, and the script fails unless its PRIVACY line passes: no private account, payer
# or query in it.
#
# Inclusion isn't success: v0.4 includes and charges a public action that fails, and only marks
# it reverted. So each attestation's outcome is read from its block, and the script fails unless
# the honest ones are accepted and every broken one is refused.
#
# The attestation proves a WETH storage slot (WETH_SLOT, a balance mapping entry) is nonzero at
# the block the instance last recorded; its eth_getProof response is recorded too.
#
# The recorded segment starts at a checkpoint of 6 Nov 2026. The program refuses an update more
# than 14 days past its store (the freshness bound, which the sequencer's clock enforces through
# the transaction's validity window), so on recorded data the updates run until about
# 20 Nov 2026. After that the run needs fresh data, which the operator daemon fetches.
#
# LC_FROM=attest skips to the attestations, on the instance the last run left in
# .devnet/lc-last. With real proofs each update takes over half an hour, so a failure after
# one needn't cost it again.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
cd "$HELLO_DIR"

CHAIN="$HELLO_DIR/tests/data/mainnet-1904"
FEES="$LOGS/fees.log"
WETH=0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2
WETH_SLOT="${WETH_SLOT:-0x390f6178407c9b8e95802b8659e6df8e34c1e3d4f8d6a49e6132bbcdd937b63a}"

# A private account's next transaction spends the state its last one created. The clients
# save that state after each private send, but a wallet that missed one (an interrupted run)
# would replay a spent commitment, which the sequencer refuses. Syncing first rules that out.
sync_private() { wallet account sync-private >/dev/null; }

# A private step's fee is reserved from the private fee payer inside the proof, and the surplus
# refunded to the payer's blinded alias as a pending credit, which the next private send claims
# back into the payer. So the reserve is the payer's drop plus what the send claimed (swept),
# the charge is the reserve minus this step's refund, and the public cycles the step ran follow
# from LEZ's private fee formula:
#   (PRIVATE_VERIFY_GAS 409,764 + public cycles)·base_fee_exec + PRIVATE_GAS_STOR 224,063·base_fee_stor
fee_payer_balance() {
    sync_private
    wallet account get --account-id "$FEE_PAYER" | grep -oE "Balance [0-9]+" | awk '{print $2}'
}
private_fee() {
    local label=$1 before=$2 after refund swept reserve charged
    after=$(fee_payer_balance)
    refund=$(grep -oE "FEE step=[a-z-]*-refund charged=-[0-9]+" <<<"$LAST" | grep -oE "[0-9]+$")
    swept=$(grep -oE "FEE step=[a-z-]*-refund charged=-[0-9]+ swept=[0-9]+" <<<"$LAST" | grep -oE "[0-9]+$")
    reserve=$(( before - after + swept ))
    charged=$(( reserve - refund ))
    # Base fees stayed at the floor of 8 throughout; the public FEE lines show it.
    local line="FEE step=$label reserve=$reserve refund=$refund swept=$swept charged=$charged public_cycles=$(( charged / 8 - 409764 - 224063 )) base_fee_exec=8 base_fee_stor=8"
    echo "  $line"
    echo "$(date -u +%FT%TZ) dev_mode=$DEV_MODE $line" >> "$FEES"
}

lc() {
    local out
    # /usr/bin/time -l reports the peak resident memory of the client and the prover it runs.
    out=$(env "${DEV_MODE_ENV[@]}" /usr/bin/time -l ./target/release/eth_lc "$@" 2>&1) \
        || { echo "$out" | grep -v WARNING | tail -20 >&2; die "eth_lc $1 failed"; }
    local rss
    rss=$(grep -oE "[0-9]+ +maximum resident set size" <<<"$out" | awk '{print $1}' || true)
    out=$(grep -vE "^ +[0-9]+ +[a-z]|^ +[0-9.]+ real" <<<"$out" || true)
    [ -n "$rss" ] && out+=$'\n'"MEM step=$1 max_rss_bytes=$rss"
    echo "$out" | grep -vE "WARNING|^Transaction is included" | sed 's/^/  /'
    # Not every command charges anything; grep finding nothing must not end the script.
    { echo "$out" | grep -E "^(FEE|TIME|MEM|STATUS|ATTEST|DRYRUN|PUBLISHED|PRIVACY)" | sed "s/^/$(date -u +%FT%TZ) dev_mode=$DEV_MODE /" >> "$FEES"; } || true
    LAST="$out"
}

# Fails unless the last private step published nothing it must keep private.
expect_private() {
    grep -qE "^PRIVACY .* pass$" <<<"$LAST" || die "private step published something private: $(grep -E '^PRIVACY' <<<"$LAST" || echo 'no PRIVACY line')"
}

# Fails unless the last public attestation's local dry run refused it, as the chain then must.
expect_dryrun_refused() {
    grep -qE "^DRYRUN .* verdict=" <<<"$LAST" && ! grep -qE "^DRYRUN .* verdict=holds$" <<<"$LAST" \
        || die "the dry run didn't refuse a broken attestation: $(grep -E '^DRYRUN' <<<"$LAST" || echo 'no DRYRUN line')"
}

# Fails unless the last attestation's outcome is one of the expected ones.
expect() {
    local outcome
    outcome=$(grep -oE "^ATTEST .*outcome=[a-z]+" <<<"$LAST" | grep -oE "[a-z]+$" || true)
    [[ " $* " == *" $outcome "* ]] || die "expected outcome $*, got ${outcome:-none}"
}

step "Building the light-client program and its client"
cargo build --release --bin eth_lc
LC_BIN=crates/program/target/riscv32im-risc0-zkvm-elf/docker/eth_lc.bin
# Always rebuilt: a stale program deployed from a previous build fails on the first message the
# client changed. The build is reproducible, so an unchanged source gives the same image.
docker info >/dev/null 2>&1 || die "Docker is not running; the guest builds in a container"
[ -f crates/program/Cargo.lock ] || cargo generate-lockfile --manifest-path crates/program/Cargo.toml
# Helios's dependencies need rustc 1.94 or later; the 1.97 builder ships amd64 only.
DOCKER_DEFAULT_PLATFORM=linux/amd64 RISC0_DOCKER_CONTAINER_TAG="$LC_BUILDER_TAG" \
    cargo risczero build --manifest-path crates/program/Cargo.toml > "$LOGS/lc-guest-build.log" 2>&1 \
    || { tail -20 "$LOGS/lc-guest-build.log" >&2; die "the light-client program didn't build"; }
{ grep -m1 "ImageID" "$LOGS/lc-guest-build.log" | sed 's/^/  /'; } || true
echo "  $LC_BIN"

step "Accounts"
# The list is read into a variable before searching it: a grep that stops early closes the
# pipe, the wallet panics writing to it, and pipefail reports that as the account missing.
ACCOUNTS=$(wallet account list)
PAYER=$(grep -oE "Public/CbgR6tj5[1-9A-HJ-NP-Za-km-z]+" <<<"$ACCOUNTS" | head -1 || true)
if [ -z "$PAYER" ]; then
    # The funded genesis account pays for public transactions.
    PAYER=$(wallet account import public --private-key "$PAYER_KEY" 2>&1 | grep -oE "Public/[1-9A-HJ-NP-Za-km-z]+" | head -1 || true)
    [ -n "$PAYER" ] || die "could not import the genesis payer"
fi
FEE_PAYER=$(grep -F "[fee-payer" <<<"$ACCOUNTS" | grep -oE "Private/[1-9A-HJ-NP-Za-km-z]+" | head -1 || true)
if [ -z "$FEE_PAYER" ]; then
    # A private transaction pays its fee inside the proof, from an owned private account, so one
    # has to hold tokens first. Shielding them from the public payer is itself a private
    # transaction.
    label="fee-payer-$(date +%s)"
    wallet account new private --label "$label" >/dev/null
    ACCOUNTS=$(wallet account list)
    FEE_PAYER=$(grep -F "[$label]" <<<"$ACCOUNTS" | grep -oE "Private/[1-9A-HJ-NP-Za-km-z]+" | head -1 || true)
    [ -n "$FEE_PAYER" ] || die "could not create the private fee payer"
    env "${DEV_MODE_ENV[@]}" "$DEVNET_BIN/wallet" auth-transfer send \
        --from "$PAYER" --to "$FEE_PAYER" --amount 100000000000 2>&1 | grep -E "included|rror" | sed 's/^/  /'
fi
if [ -n "${LC_FROM:-}" ]; then
    [ -f "$DEVNET/lc-last" ] || die "LC_FROM=$LC_FROM needs a previous run's .devnet/lc-last"
    read -r PROGRAM INSTANCE OPERATOR <"$DEVNET/lc-last"
else
    stamp=$(date +%s)
    wallet account new public --label "lc-instance-$stamp" >/dev/null
    wallet account new private --label "lc-operator-$stamp" >/dev/null
    ACCOUNTS=$(wallet account list)
    INSTANCE=$(grep -F "[lc-instance-$stamp]" <<<"$ACCOUNTS" | grep -oE "Public/[1-9A-HJ-NP-Za-km-z]+")
    OPERATOR=$(grep -F "[lc-operator-$stamp]" <<<"$ACCOUNTS" | grep -oE "Private/[1-9A-HJ-NP-Za-km-z]+")
fi
echo "  payer $PAYER"
echo "  instance $INSTANCE"
echo "  operator $OPERATOR"
echo "  private fee payer $FEE_PAYER"

if [ -z "${LC_FROM:-}" ]; then
step "Checking the recorded checkpoint and updates natively"
lc check "$CHAIN"

step "Deploying eth_lc (immutable)"
lc deploy "$LC_BIN" "$PAYER"
PROGRAM=$(grep -oE "header account [1-9A-HJ-NP-Za-km-z]+" <<<"$LAST" | awk '{print $3}')
echo "$PROGRAM $INSTANCE $OPERATOR" >"$DEVNET/lc-last"

step "Init: binding the config and the checkpoint (public, signed by the instance)"
lc init "$PROGRAM" "$INSTANCE" "$CHAIN" "$PAYER"

# Each update the recorded segment holds, in order: every one is signed by a committee the
# store already holds, so the instance follows the handoffs to the finality update.
n=0
while :; do
    lc pending "$PROGRAM" "$INSTANCE" "$CHAIN"
    grep -q "next update verifies" <<<"$LAST" || break
    n=$((n + 1))
    handoff=$(grep -oE "handoff (true|false)" <<<"$LAST" | awk '{print $2}')
    step "Update $n: $([ "$handoff" = true ] && echo "committee handoff" || echo finality) (private, proven locally)"
    FP=$(fee_payer_balance)
    lc update "$LC_BIN" "$PROGRAM" "$INSTANCE" "$CHAIN" "$OPERATOR" "$PAYER"
    private_fee lc-update-charged "$FP"
    expect_private
done
[ "$n" -gt 0 ] || die "no recorded update advances the instance"
fi
lc show "$PROGRAM" "$INSTANCE"
BLOCK=$(grep -oE "latest block [0-9]+" <<<"$LAST" | awk '{print $3}')
PROOF="$CHAIN/getproof-$BLOCK.json"
[ -f "$PROOF" ] || die "no recorded storage proof at block $BLOCK ($PROOF)"

step "Public attestation"
lc attest-public "$PROGRAM" "$INSTANCE" "$PROOF" "$PAYER"
expect accepted

step "Private attestation (proven locally; only the record check is public)"
FP=$(fee_payer_balance)
lc attest-private "$LC_BIN" "$PROGRAM" "$INSTANCE" "$PROOF" "$OPERATOR" "$PAYER"
private_fee lc-attest-private-charged "$FP"
expect accepted
expect_private

step "Negative attestations: each breaks one thing, and each must be refused"
# On the public path the sequencer runs the check, so a broken attestation is included and
# charged, then reverted. On the private path the wallet runs the check before proving, so a
# broken proof or claim can't be proven at all. A record the instance doesn't hold passes
# the caller's own checks, and only the instance's CheckRecord refuses it. The wallet runs that
# public call too before proving, so an honest wallet stops there (unprovable). A wallet that
# skipped that step would reach the sequencer, which runs CheckRecord itself; this script
# doesn't test that path.
for case in tampered-proof false-claim; do
    lc attest-public "$PROGRAM" "$INSTANCE" "$PROOF" "$PAYER" "$case"
    expect_dryrun_refused
    expect reverted
    sync_private
    lc attest-private "$LC_BIN" "$PROGRAM" "$INSTANCE" "$PROOF" "$OPERATOR" "$PAYER" "$case"
    expect unprovable
done
sync_private
lc attest-private "$LC_BIN" "$PROGRAM" "$INSTANCE" "$PROOF" "$OPERATOR" "$PAYER" unheld-record
expect unprovable reverted rejected
printf '\n\033[1;32mPASS\033[0m  eth_lc followed recorded mainnet on LEZ through the committee handoffs and attested a storage slot publicly and privately; every broken attestation was refused.\n'
echo "Fees and times: $FEES"
