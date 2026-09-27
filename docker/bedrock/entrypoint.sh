#!/bin/sh
# Runs bedrock as a single-validator devnet. node-config.yaml and
# deployment-settings.yaml carry matching validator keys, so one node both proposes
# and finalises blocks.
set -e

# Proof-of-leadership proofs are stubbed. This is a devnet; never set this on a
# network whose blocks you care about.
export POL_PROOF_DEV_MODE=true

exec /usr/bin/logos-blockchain-node \
    /etc/logos-blockchain/node-config.yaml \
    --deployment /etc/logos-blockchain/deployment-settings.yaml
