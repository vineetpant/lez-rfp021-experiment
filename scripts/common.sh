# Shared settings for the LEZ hello-world scripts. Sourced by bash, not executed.

HELLO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The LEZ release everything is pinned to: the crates this project builds against, and
# the service binaries the devnet runs. Bedrock's version is separate, in
# docker-compose.yml, because it comes from a different repo's releases.
LEZ_TAG="${LEZ_TAG:-v0.3.0-rc1}"
LEZ_GIT="${LEZ_GIT:-https://github.com/logos-blockchain/logos-execution-zone.git}"

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

# The first genesis supply account in configs/sequencer_config.json. Freshly claimed
# accounts hold no balance, so something funded has to pay for claiming them.
PAYER_KEY=7f273098f25b71e6c005a9519f2678da8d1c7f01f6a27778e2d9948abdf901fb
PAYER_ID=CbgR6tj5kWx5oziiFptM7jMvrQeYY3Mzaao6ciuhSr2r

export PATH="$PATH:$HOME/.risc0/bin"
export LEE_WALLET_HOME_DIR="$DEVNET/wallet"
# Without this the wallet prints ~1.2MB of raw transaction bytes per call.
export SUPPRESS_VERBOSE_PRINTS=1

# RISC0_DEV_MODE is deliberately not exported: it is part of the environment risc0's
# build scripts hash, so exporting it invalidates them and forces guest rebuilds on every
# cargo invocation. Pass it only to processes that execute programs.
DEV_MODE_ENV=(RISC0_DEV_MODE=1)

die() { echo "error: $*" >&2; exit 1; }
step() { printf '\n\033[1;34m==>\033[0m %s\n' "$1"; }

wallet() { printf '%s\n' "$WALLET_PASSWORD" | "$DEVNET_BIN/wallet" "$@"; }

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
