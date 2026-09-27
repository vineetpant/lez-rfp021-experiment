# Bedrock, the Logos base chain that LEZ posts its blocks to.
#
# The node binary comes from a published GitHub release rather than being built from
# source. Upstream's own resolver insists on the exact logos-blockchain revision that
# LEZ's Cargo.lock pins, and source-builds when no release matches -- which costs about
# eight minutes and needs the LEZ checkout. A release binary makes this image
# self-contained and quick.
#
# The tradeoff: LEZ may pin a revision newer than the newest release, so this node can
# be slightly behind what LEZ was tested against. For a local hello-world devnet that is
# fine. If the sequencer starts rejecting blocks or the channel misbehaves after a LEZ
# upgrade, that mismatch is the first thing to suspect -- use upstream's
# `just resolve-bedrock-node` to build the exact pinned revision instead.
FROM debian:trixie-slim

# Latest release at the time of writing. Override with --build-arg NODE_VERSION=...
ARG NODE_VERSION=0.3.0-rc.4
# Set automatically by buildkit: arm64 or amd64.
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libssl3t64 \
        libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    case "${TARGETARCH:-arm64}" in \
        arm64) asset_arch=linux-aarch64 ;; \
        amd64) asset_arch=linux-x86_64 ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    asset="logos-blockchain-node-${asset_arch}-${NODE_VERSION}.tar.gz"; \
    url="https://github.com/logos-blockchain/logos-blockchain/releases/download/${NODE_VERSION}/${asset}"; \
    echo "fetching ${url}"; \
    curl -fsSL "${url}" -o /tmp/node.tar.gz; \
    tar -xzf /tmp/node.tar.gz -C /usr/bin logos-blockchain-node; \
    chmod 0755 /usr/bin/logos-blockchain-node; \
    rm /tmp/node.tar.gz; \
    /usr/bin/logos-blockchain-node --version || true

# Static single-validator configuration, copied from logos-execution-zone's bedrock/.
COPY bedrock/node-config.yaml bedrock/deployment-settings.yaml /etc/logos-blockchain/
# KZG parameters. Upstream mounts this at the filesystem root, so the node finds it
# there; keep the path.
COPY bedrock/kzgrs_test_params /kzgrs_test_params
COPY bedrock/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 0755 /usr/local/bin/entrypoint.sh

# The API the LEZ sequencer and indexer talk to; node-config.yaml binds 0.0.0.0:18080.
EXPOSE 18080

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
