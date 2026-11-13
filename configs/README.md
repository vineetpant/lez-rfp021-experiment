# Devnet configuration

These came from `logos-execution-zone`'s debug configs, copied here so this repo can run a
devnet without needing that checkout. They describe a one-sequencer, one-validator local
network. Don't use them for anything else — the keys are public and the balances are made up.

| File | Copied from | What to know |
|---|---|---|
| `sequencer_config.json` | `lez/sequencer/service/configs/debug/` | `home: "."`, so RocksDB lands wherever the service was started — hence the separate state dirs |
| `indexer_config.json` | `lez/indexer/service/configs/debug/` | points at bedrock on `localhost:18080` |
| `wallet/wallet_config.json` | `lez/wallet/configs/debug/` | points at the sequencer on `127.0.0.1:3040` |

`sequencer_config.json` also holds the genesis accounts. `scripts/env.sh` uses the first of
them to pay transaction fees:

```
CbgR6tj5kWx5oziiFptM7jMvrQeYY3Mzaao6ciuhSr2r   balance 10000000000000
```

The sequencer and indexer both name channel `0101…01`. If those ever stop matching, the indexer
will sit there reading a channel nobody writes to.
