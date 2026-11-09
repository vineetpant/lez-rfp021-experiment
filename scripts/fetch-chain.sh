#!/usr/bin/env bash
# Fetch a live mainnet chain segment for eth_lc, as Beacon API JSON the client decodes:
#
#   ./scripts/fetch-chain.sh <dir>            bootstrap at the current finalized checkpoint,
#                                             the period's best update, the finality update
#   ./scripts/fetch-chain.sh <dir> refresh    the finality update only, to advance past the
#                                             bootstrap once at least an epoch has finalized
#   ./scripts/fetch-chain.sh <dir> proof <address> <slot>
#                                             eth_getProof for one storage slot at the block
#                                             the instance last recorded (BLOCK=<n>)
#   ./scripts/fetch-chain.sh <dir> header <n>   block n's header, as JSON and as RLP
#   ./scripts/fetch-chain.sh <dir> history <anchor> <target>
#                                             the EIP-2935 history contract's slot for block
#                                             <target>, proven at block <anchor>, and the
#                                             target's header
#
# BEACON_API must serve the light-client routes. EXEC_RPC is a space-separated list of nodes
# tried in turn for eth_getProof at the recorded block. With real proofs that block is an hour
# or more behind head, and free tiers keep only recent state: drpc's forgets it within about
# an hour, and PublicNode refuses anything outside its proof window.
set -euo pipefail
DIR=${1:?usage: fetch-chain.sh <dir> [refresh | proof <address> <slot>]}
MODE=${2:-full}
BEACON_API="${BEACON_API:-https://ethereum-beacon-api.publicnode.com}"
EXEC_RPC="${EXEC_RPC:-https://rpc.mevblocker.io https://eth-mainnet.public.blastapi.io https://eth.drpc.org}"
mkdir -p "$DIR"

get() { curl -fsS -m 30 "$BEACON_API$1"; }

# One JSON-RPC call, tried on each node in EXEC_RPC until one returns a result, into $OUT.
rpc() {
    local body=$1 node
    for node in $EXEC_RPC; do
        curl -fsS -m 30 -X POST "$node" -H 'content-type: application/json' -d "$body" > "$OUT" 2>/dev/null \
            && python3 -c "import json,sys; d=json.load(open(sys.argv[1])); assert d.get('result')" "$OUT" 2>/dev/null \
            && { RPC=$node; return 0; }
        echo "$node didn't answer $(python3 -c "import json,sys; print(json.loads(sys.argv[1])['method'])" "$body"), trying the next node" >&2
        rm -f "$OUT"
    done
    echo "no node in EXEC_RPC answered" >&2
    exit 1
}

header() {
    local n=$1
    OUT="$DIR/header-$n.json"
    rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"$(printf '0x%x' "$n")\",false]}"
    python3 -I "$(dirname "${BASH_SOURCE[0]}")/header_rlp.py" "$OUT" > "$DIR/header-$n.rlp"
}
field() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

finality() {
    get /eth/v1/beacon/light_client/finality_update > "$DIR/finality_update.json"
    echo "finality update: finalized slot $(field "d['data']['finalized_header']['beacon']['slot']" < "$DIR/finality_update.json"), block $(field "d['data']['finalized_header']['execution']['block_number']" < "$DIR/finality_update.json")"
}

case "$MODE" in
    full)
        rm -f "$DIR"/bootstrap-*.json "$DIR"/bootstrap-*.root "$DIR"/updates-*.json
        get /eth/v1/beacon/genesis > "$DIR/genesis.json"
        get /eth/v1/config/fork_schedule > "$DIR/fork_schedule.json"
        ROOT=$(get /eth/v1/beacon/states/head/finality_checkpoints | field "d['data']['finalized']['root']")
        get "/eth/v1/beacon/light_client/bootstrap/$ROOT" > "$DIR/bootstrap.tmp"
        SLOT=$(field "d['data']['header']['beacon']['slot']" < "$DIR/bootstrap.tmp")
        mv "$DIR/bootstrap.tmp" "$DIR/bootstrap-$SLOT.json"
        echo "$ROOT" > "$DIR/bootstrap-$SLOT.root"
        PERIOD=$((SLOT / 8192))
        get "/eth/v1/beacon/light_client/updates?start_period=$PERIOD&count=1" > "$DIR/updates-$PERIOD.json"
        echo "bootstrap: slot $SLOT, period $PERIOD, root $ROOT"
        finality
        ;;
    refresh)
        finality
        ;;
    proof)
        ADDRESS=${3:?address}; KEY=${4:?storage slot}; BLOCK=${BLOCK:?BLOCK=<block number> to prove at}
        OUT="$DIR/getproof-$BLOCK.json"
        BODY="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getProof\",\"params\":[\"$ADDRESS\",[\"$KEY\"],\"$(printf '0x%x' "$BLOCK")\"]}"
        for RPC in $EXEC_RPC; do
            curl -fsS -m 30 -X POST "$RPC" -H 'content-type: application/json' -d "$BODY" > "$OUT" 2>/dev/null \
                && python3 -c "import json,sys; d=json.load(open(sys.argv[1])); assert 'result' in d" "$OUT" 2>/dev/null \
                && break
            echo "$RPC has no proof at block $BLOCK, trying the next node" >&2
            rm -f "$OUT"
        done
        [ -f "$OUT" ] || { echo "no node in EXEC_RPC served eth_getProof at block $BLOCK" >&2; exit 1; }
        python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print('proof from', sys.argv[2] + ':', len(d['result']['accountProof']), 'account nodes,', len(d['result']['storageProof'][0]['proof']), 'storage nodes, value', d['result']['storageProof'][0]['value'])" "$OUT" "$RPC"
        ;;
    header)
        N=${3:?block number}
        header "$N"
        echo "header $N: $(wc -c < "$DIR/header-$N.rlp" | awk '{print ($1 - 1) / 2}') bytes of RLP"
        ;;
    history)
        ANCHOR=${3:?anchor block}; TARGET=${4:?target block}
        SLOT=$(printf '0x%064x' $((TARGET % 8191)))
        OUT="$DIR/history-$ANCHOR-$TARGET.json"
        rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getProof\",\"params\":[\"0x0000F90827F1C53a10cb7A02335B175320002935\",[\"$SLOT\"],\"$(printf '0x%x' "$ANCHOR")\"]}"
        echo "history slot for block $TARGET at block $ANCHOR, from $RPC: $(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['result']['storageProof'][0]['value'])" "$OUT")"
        header "$TARGET"
        ;;
    *) echo "unknown mode $MODE" >&2; exit 2 ;;
esac
