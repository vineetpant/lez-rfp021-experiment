# Shared settings for the LEZ hello-world scripts. Sourced by bash, not executed.

HELLO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The LEZ release everything is pinned to: the crates this project builds against, and
# the service binaries the devnet runs.
LEZ_TAG="${LEZ_TAG:-v0.4.0-rc1}"
LEZ_GIT="${LEZ_GIT:-https://github.com/logos-blockchain/logos-execution-zone.git}"

# Bedrock, pinned to the exact commit LEZ_TAG's Cargo.lock resolves. No published release
# matches it (it has diverged from 0.3.4), so it is built from source, the same way LEZ's
# own resolver falls back. Bump it with LEZ_TAG, from that tag's Cargo.lock.
BEDROCK_REV="${BEDROCK_REV:-d480efc6bbb48fb493c10fe9afefd84c18a04f02}"
BEDROCK_GIT="${BEDROCK_GIT:-https://github.com/logos-blockchain/logos-blockchain.git}"

# Everything the devnet writes lives here, so a reset is a directory removal and nothing
# leaks into the repo or a neighbouring checkout.
DEVNET="$HELLO_DIR/.devnet"
DEVNET_BIN="$DEVNET/bin"
LOGS="${LOGS:-$DEVNET/logs}"

# Devnet wallet password. Seeds all account keys deterministically; throwaway.
WALLET_PASSWORD="${WALLET_PASSWORD:-program-tutorial}"

# Guest-builder image tag. The default tag ships a rustc too old for this dependency
# tree; this is the tag logos-execution-zone's own Justfile pins.
GUEST_BUILDER_TAG="${GUEST_BUILDER_TAG:-r0.1.91.1}"

# The private key of the first genesis supply account in configs/sequencer_network.json.
# Freshly claimed accounts hold no balance, so something funded has to pay for claiming them.
PAYER_KEY=7f273098f25b71e6c005a9519f2678da8d1c7f01f6a27778e2d9948abdf901fb

# The sequencer's Bedrock funding key, matching configs/bedrock/node-config.yaml.
BEDROCK_FUNDING_PUBLIC_KEY=2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26

export PATH="$PATH:$HOME/.risc0/bin"
export LEE_WALLET_HOME_DIR="$DEVNET/wallet"
# Without this the wallet prints ~1.2MB of raw transaction bytes per call.
export SUPPRESS_VERBOSE_PRINTS=1

# DEV_MODE=1 (the default) fakes every proof: quick, and fine for checking the plumbing. A
# node in dev mode accepts fake proofs, so it verifies nothing. Every measured run uses
# DEV_MODE=0, set the same for the devnet and the client, because a real node refuses a
# fake proof.
#
# RISC0_DEV_MODE is deliberately not exported: it is part of the environment risc0's
# build scripts hash, so exporting it invalidates them and forces guest rebuilds on every
# cargo invocation. Pass it only to processes that execute programs.
DEV_MODE="${DEV_MODE:-1}"
DEV_MODE_ENV=(RISC0_DEV_MODE="$DEV_MODE")

die() { echo "error: $*" >&2; exit 1; }
step() { printf '\n\033[1;34m==>\033[0m %s\n' "$1"; }

# The v0.4 wallet reads its password from a terminal or from LEE_WALLET_PASSWORD, not stdin.
export LEE_WALLET_PASSWORD="$WALLET_PASSWORD"
wallet() { "$DEVNET_BIN/wallet" "$@"; }

# Installs wallet, sequencer_service and indexer_service from the pinned tag into
# .devnet/bin. Local rather than ~/.cargo/bin so it cannot collide with a wallet the
# user already has, and so removing this repo removes them.
ensure_devnet_binaries() {
    local missing=0 b
    for b in wallet sequencer_service indexer_service; do
        [ -x "$DEVNET_BIN/$b" ] || missing=1
    done
    if [ "$missing" -eq 0 ]; then
        echo "  already installed in .devnet/bin"
        return 0
    fi
    echo "  installing from $LEZ_TAG - first run takes ~8 min"
    cargo install --git "$LEZ_GIT" --tag "$LEZ_TAG" --locked \
        --root "$DEVNET" wallet sequencer_service indexer_service
}

# Builds the Bedrock node at BEDROCK_REV into .devnet/bin. The `testing` feature is what
# LEZ's resolver builds with; it lets one validator produce blocks on its own.
ensure_bedrock_binary() {
    if [ -x "$DEVNET_BIN/logos-blockchain-node" ]; then
        echo "  already installed in .devnet/bin"
        return 0
    fi
    echo "  building ${BEDROCK_REV:0:9} from source - first run takes ~9 min"
    cargo install --git "$BEDROCK_GIT" --rev "$BEDROCK_REV" --locked --features testing \
        --root "$DEVNET" logos-blockchain-node
}
