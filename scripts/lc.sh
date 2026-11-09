#!/usr/bin/env bash
# The light client end to end on the local devnet, on live mainnet data:
#
#   deploy eth_lc -> Init from a live checkpoint -> private update (committee handoff)
#   -> private update (finality) -> public attestation -> private attestation
#   -> attestations to older blocks: EIP-2935 and a parent-hash chain
#   -> negative attestations, which must fail
#
# Run ./scripts/up.sh first, and ./scripts/hello.sh once (it funds a private fee payer).
# Every charge is a FEE line, every proven step a TIME line, and every transaction's outcome a
# STATUS line, all kept in .devnet/logs/fees.log. DEV_MODE=0, for up.sh too, proves for real.
# After each private step, PUBLISHED lines list what its transaction made public, and the
# script fails unless its PRIVACY line passes: no private account, payer or query in it.
#
# Inclusion isn't success: v0.4 includes and charges a public action that fails, and only marks
# it reverted. So each attestation's outcome is read from its block, and the script fails unless
# the honest ones are accepted and every broken one is refused.
#
# The attestation proves a WETH storage slot (WETH_SLOT, a balance mapping entry) is
# nonzero at the block the instance last recorded.
#
# LC_FROM=update2 skips to the second update, LC_FROM=attest to the attestations, and
# LC_FROM=history to the attestations to older blocks, on the
# instance the last run left in .devnet/lc-last. With real proofs each update takes over half
# an hour, so a failure after one needn't cost it again.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$HELLO_DIR"

CHAIN="$DEVNET/chain"
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
LC_BIN=lc/guest/target/riscv32im-risc0-zkvm-elf/docker/eth_lc.bin
# Always rebuilt: a stale program deployed from a previous build fails on the first message the
# client changed. The build is reproducible, so an unchanged source gives the same image.
docker info >/dev/null 2>&1 || die "Docker is not running; the guest builds in a container"
[ -f lc/guest/Cargo.lock ] || cargo generate-lockfile --manifest-path lc/guest/Cargo.toml
# Helios's dependencies need rustc 1.94 or later; the 1.97 builder ships amd64 only.
DOCKER_DEFAULT_PLATFORM=linux/amd64 RISC0_DOCKER_CONTAINER_TAG="$LC_BUILDER_TAG" \
    cargo risczero build --manifest-path lc/guest/Cargo.toml > "$LOGS/lc-guest-build.log" 2>&1 \
    || { tail -20 "$LOGS/lc-guest-build.log" >&2; die "the light-client program didn't build"; }
{ grep -m1 "ImageID" "$LOGS/lc-guest-build.log" | sed 's/^/  /'; } || true
echo "  $LC_BIN"

step "Accounts"
# The list is read into a variable before searching it: a grep that stops early closes the
# pipe, the wallet panics writing to it, and pipefail reports that as the account missing.
ACCOUNTS=$(wallet account list)
PAYER=$(grep -oE "Public/CbgR6tj5[1-9A-HJ-NP-Za-km-z]+" <<<"$ACCOUNTS" | head -1 || true)
[ -n "$PAYER" ] || die "no genesis payer in the wallet - run ./scripts/hello.sh first"
FEE_PAYER=$(grep -F "[fee-payer" <<<"$ACCOUNTS" | grep -oE "Private/[1-9A-HJ-NP-Za-km-z]+" | head -1 || true)
[ -n "$FEE_PAYER" ] || die "no private fee payer - run ./scripts/hello.sh first"
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
step "Fetching a live mainnet checkpoint and checking it natively"
./scripts/fetch-chain.sh "$CHAIN" | sed 's/^/  /'
lc check "$CHAIN"

step "Deploying eth_lc (immutable)"
lc deploy "$LC_BIN" "$PAYER"
PROGRAM=$(grep -oE "header account [1-9A-HJ-NP-Za-km-z]+" <<<"$LAST" | awk '{print $3}')
echo "$PROGRAM $INSTANCE $OPERATOR" >"$DEVNET/lc-last"

step "Init: binding the config and the checkpoint (public, signed by the instance)"
lc init "$PROGRAM" "$INSTANCE" "$CHAIN" "$PAYER"

step "Update 1: committee handoff (private, proven locally)"
FP=$(fee_payer_balance)
lc update "$LC_BIN" "$PROGRAM" "$INSTANCE" "$CHAIN" "$OPERATOR" "$PAYER"
private_fee lc-update-charged "$FP"
expect_private
fi

if [ -z "${LC_FROM:-}" ] || [ "$LC_FROM" = update2 ]; then
step "Update 2: finality (private, proven locally)"
# The finality update has to finalize past the instance's header, which takes at least an
# epoch. It is checked against the instance as it stands on the node, not a fresh bootstrap.
for i in $(seq 1 30); do
    ./scripts/fetch-chain.sh "$CHAIN" refresh | sed 's/^/  /'
    lc pending "$PROGRAM" "$INSTANCE" "$CHAIN"
    read -r FROM TO < <(grep -oE "slot [0-9]+ -> [0-9]+" <<<"$LAST" | awk '{print $2, $4}') || true
    [ -n "${TO:-}" ] && [ "$TO" -gt "$FROM" ] && break
    [ "$i" -eq 30 ] && die "no finality update past the checkpoint after 30 minutes"
    sleep 60
done
FP=$(fee_payer_balance)
lc update "$LC_BIN" "$PROGRAM" "$INSTANCE" "$CHAIN" "$OPERATOR" "$PAYER"
private_fee lc-update-charged "$FP"
expect_private
fi
lc show "$PROGRAM" "$INSTANCE"
BLOCK=$(grep -oE "latest block [0-9]+" <<<"$LAST" | awk '{print $3}')

step "Fetching a WETH storage proof at block $BLOCK"
BLOCK=$BLOCK ./scripts/fetch-chain.sh "$CHAIN" proof "$WETH" "$WETH_SLOT" | sed 's/^/  /'

if [ "${LC_FROM:-}" != history ]; then
step "Public attestation"
lc attest-public "$PROGRAM" "$INSTANCE" "$CHAIN/getproof-$BLOCK.json" "$PAYER"
expect accepted

step "Private attestation (proven locally; only the record check is public)"
FP=$(fee_payer_balance)
lc attest-private "$LC_BIN" "$PROGRAM" "$INSTANCE" "$CHAIN/getproof-$BLOCK.json" "$OPERATOR" "$PAYER"
private_fee lc-attest-private-charged "$FP"
expect accepted
expect_private
fi

step "Attestations to older blocks, through the latest record"
# EIP-2935: the history contract's slot for a block HISTORY_GAP back, proven under the record's
# state root, then that block's header. The parent-hash chain: CHAIN_GAP headers back from the
# record's own block. Each attests the same WETH slot, at the older block.
OLD=$((BLOCK - ${HISTORY_GAP:-1000})); NEAR=$((BLOCK - ${CHAIN_GAP:-3}))
./scripts/fetch-chain.sh "$CHAIN" history "$BLOCK" "$OLD" | sed 's/^/  /'
BLOCK=$OLD ./scripts/fetch-chain.sh "$CHAIN" proof "$WETH" "$WETH_SLOT" | sed 's/^/  /'
# A counting loop, not seq: macOS's seq prints block numbers this large as 2.6e+07.
for ((n = NEAR; n <= BLOCK; n++)); do ./scripts/fetch-chain.sh "$CHAIN" header "$n" | sed 's/^/  /'; done
BLOCK=$NEAR ./scripts/fetch-chain.sh "$CHAIN" proof "$WETH" "$WETH_SLOT" | sed 's/^/  /'
for pair in "history $OLD" "history-chain $NEAR"; do
    read -r case at <<<"$pair"
    lc attest-public "$PROGRAM" "$INSTANCE" "$CHAIN/getproof-$at.json" "$PAYER" "$case"
    expect accepted
    sync_private
    FP=$(fee_payer_balance)
    lc attest-private "$LC_BIN" "$PROGRAM" "$INSTANCE" "$CHAIN/getproof-$at.json" "$OPERATOR" "$PAYER" "$case"
    private_fee "lc-attest-private-$case-charged" "$FP"
    expect accepted
    expect_private
done

step "Negative attestations: each breaks one thing, and each must be refused"
# On the public path the sequencer runs the check, so a broken attestation is included and
# charged, then reverted. On the private path the wallet runs the check before proving, so a
# broken proof or claim can't be proven at all. A record the instance doesn't hold passes
# the caller's own checks, and only the instance's CheckRecord refuses it. The wallet runs that
# public call too before proving, so an honest wallet stops there (unprovable). A wallet that
# skipped that step would reach the sequencer, which runs CheckRecord itself; this script
# doesn't test that path.
PROOF="$CHAIN/getproof-$BLOCK.json"
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
# A header that doesn't hash to what the history contract holds for its block.
lc attest-public "$PROGRAM" "$INSTANCE" "$CHAIN/getproof-$OLD.json" "$PAYER" history-forged
expect_dryrun_refused
expect reverted
sync_private
lc attest-private "$LC_BIN" "$PROGRAM" "$INSTANCE" "$CHAIN/getproof-$OLD.json" "$OPERATOR" "$PAYER" history-forged
expect unprovable

printf '\n\033[1;32mPASS\033[0m  eth_lc tracked live mainnet on LEZ and attested a storage slot publicly and privately, at the recorded block and at older ones; every broken attestation was refused.\n'
echo "Fees and times: $FEES"
