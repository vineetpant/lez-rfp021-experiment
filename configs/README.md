# Devnet configuration

Copied from `logos-execution-zone`'s debug configs so this repo can run a devnet without
that checkout. They describe a single-sequencer, single-validator local network and are
**not** safe for anything else: the keys are public and the genesis balances imaginary.

| File | From | Notes |
|---|---|---|
| `sequencer_config.json` | `lez/sequencer/service/configs/debug/` | `home: "."` — RocksDB lands in the working directory, so each service runs from its own state dir |
| `indexer_config.json` | `lez/indexer/service/configs/debug/` | points at bedrock on `localhost:18080` |
| `wallet/wallet_config.json` | `lez/wallet/configs/debug/` | points at the sequencer on `127.0.0.1:3040` |

`sequencer_config.json` carries the genesis supply accounts. `scripts/common.sh` uses the
first as the fee payer:

```
CbgR6tj5kWx5oziiFptM7jMvrQeYY3Mzaao6ciuhSr2r   balance 10000000000000
```

Both files name channel `0101…01`, which must match or the indexer reads a channel the
sequencer never writes.
