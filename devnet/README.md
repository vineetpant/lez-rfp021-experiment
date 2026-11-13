# Devnet

A local LEZ `v0.4.0-rc1` network: Bedrock, one sequencer and the indexer, all native. It is
where `eth_lc` runs end to end on recorded mainnet.

```bash
./devnet/up.sh              # build what's missing, then start Bedrock, the indexer and the sequencer
./devnet/run.sh             # the whole flow on recorded mainnet, ending in PASS
./devnet/down.sh            # stop it, keeping its state
./devnet/down.sh --reset    # stop it and wipe it back to genesis
```

`DEV_MODE=0`, for both `up.sh` and `run.sh`, proves for real. Everything the devnet writes
(binaries, chain state, wallet, logs) goes to `.devnet/` at the repository root, which git
ignores.

| File | What |
|---|---|
| `env.sh` | shared settings: the pinned LEZ and Bedrock versions, paths, the keys below |
| `up.sh`, `run.sh`, `down.sh` | start, run the flow, stop |
| `configs/bedrock/` | the Bedrock node and its deployment settings |
| `configs/sequencer_network.json` | the sequencer's network and genesis; `up.sh` writes the sequencer's own config from it once |
| `configs/indexer_config.json` | the indexer, reading Bedrock on `localhost:18080` |
| `configs/wallet/wallet_config.json` | the wallet, pointing at the sequencer on `127.0.0.1:3040` |

The configs come from `logos-execution-zone`'s debug configs, copied here so the devnet runs
without that checkout. Don't use them for anything else: the keys are public and the balances
made up.

`env.sh` pays fees from the first genesis supply account in `sequencer_network.json`, and its
Bedrock funding key matches `configs/bedrock/node-config.yaml`.

The sequencer and the indexer both name channel `0101…01`. If those ever stop matching, the
indexer reads a channel nobody writes to.
