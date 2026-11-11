# Trustless Ethereum state attestation on LEZ

A LEZ program, `eth_lc`, that follows finalized Ethereum through the sync committee and lets
other LEZ programs prove facts about Ethereum (a storage slot, an account, an event log) in
their own transaction, publicly or privately. It is our working ground for
[RFP-021](https://github.com/logos-co/rfp), on LEZ `v0.4.0-rc1`. The proposal is
[`rfp-021/proposal.md`](rfp-021/proposal.md).

## How it fits together

```
operator (off-chain)                      LEZ: the eth_lc program
────────────────────                      ───────────────────────
fetch a light-client update
prove `Update` privately:   ── proof ──▶  verify the proof (flat fee)
  BLS signature, 2/3 of the               Advance the instance: only from eth_lc,
  committee, finality and                 only on the store and config it holds
  handoff branches                        WriteRecord: the header, once, in its own account

consumer
────────
eth_getProof / receipts
send a claim:               ──────────▶  check the proofs against a finalised header,
  facts: proofs + conditions              evaluate the conditions, return a statement with
                                          an ID per fact
  (public, or private: only               (private: only CheckRecord is published)
  CheckRecord is published)
```

1. **The light-client check** is one function, `core::update` (`lc/types/src/core.rs`): the
   consensus specs' protocol, with the BLS signature checked by `blst`.
2. **The operator proves it privately** and submits the proof. The sequencer verifies it
   without re-running the BLS check (`Update` in `lc/guest/src/bin/eth_lc.rs`).
3. **The instance advances**, and the finalised header is written once to its own account,
   derived from the instance and the execution block number (`Advance`, `WriteRecord`).
4. **A consumer sends a claim**: facts, each a Merkle-Patricia proof against a finalised header
   with conditions on its values, such as `StorageValue`, `AccountBalance` or `LogDataWord`
   ([`rfp-021/statements.md`](rfp-021/statements.md)).
5. **`statement::check`** verifies the proofs, evaluates the conditions and returns a statement
   with one ID per fact, from the Ethereum state alone, for the consumer's nullifier.

The verifier is held to Helios's `consensus-core` in the tests: on every recorded update and
every attack case, both reach the same verdict and store (proposal section 11).

## What it costs

Each message run through the program in RISC Zero's executor, metered as LEZ meters it
(`cargo test --release --test cost -- --nocapture`):

| Action | Path | Cycles | Within |
|---|---|---:|---|
| Update, committee handoff | private, proven by the operator | 12,554,439 | not metered: a flat fee |
| Update, finality | private, proven by the operator | 11,369,739 | not metered: a flat fee |
| Init | public | 1,359,479 | 10M per transaction |
| Attestation, storage slot | public | 301,471 | 2M default gas limit |
| Attestation, account balance | public | 179,974 | 2M default gas limit |
| Attestation, receipt log | public | 83,076 | 2M default gas limit |
| Each extra condition | public | 1,486 | 36 B more |

On a LEZ v0.4 devnet with real proofs (9 Nov 2026, M3 Pro laptop, base fees at the floor of 8;
[`rfp-021/results.md`](rfp-021/results.md)):

| Action | Time | Peak memory | Charge |
|---|---:|---:|---:|
| Init | one block | — | 11,194,336 |
| Update, committee handoff | 36.7 min proving | 8.4 GB | 5,333,856 |
| Update, finality | 35.6 min proving | 8.7 GB | 5,526,600 |
| Attestation, public, storage slot | one block | — | 2,469,032 |
| Attestation, private, storage slot | 14.6 min proving | 7.3 GB | 5,232,864 |
| Attestation, public, refused | one block | — | 16,060,112 (its whole gas limit) |

A private transaction's fee is flat: (409,764 + its public cycles) × 8 + 224,063 × 8. The
proving work shows up as time on the prover's machine, not as fees.

**Not measured yet.** Every attestation run on the node was a storage-slot claim. Account and
receipt-log claims are tested natively and their cycles measured above, but they haven't run on
the node, so their transaction size and their private proving time and memory are still to
measure ([results.md](rfp-021/results.md#what-these-numbers-cover)).

## Tested against recorded mainnet

Init and the light-client updates run against recorded mainnet data in
[`tests/data/mainnet-1904`](tests/data/mainnet-1904): a checkpoint at slot 15,597,568, the
updates through two committee handoffs, a finality update, a WETH `eth_getProof` and the
block's receipts. The same data drives both:

- **the native tests**: the program's code against Helios, the attack cases, attestations,
  history proofs and the cycle costs;
- **the devnet run** (`scripts/lc.sh`): deploy, Init, every update proven and submitted, public
  and private attestations, and the broken attestations, each of which must be refused.

The program refuses an update more than 14 days past its store, the freshness bound that the
sequencer's clock enforces. So `lc.sh` on this data runs until about **20 Nov 2026**; after
that the devnet needs fresh data, which the operator daemon will fetch. The native tests don't
depend on the clock and keep passing.

## Run it

```bash
cargo test --release                               # every native test
cargo test --release --test cost -- --nocapture    # the cycle table above
```

The cost test needs the program built first, in a pinned container (Docker, and the RISC Zero
toolchain: `curl -L https://risczero.com/install | bash && rzup install`):

```bash
DOCKER_DEFAULT_PLATFORM=linux/amd64 RISC0_DOCKER_CONTAINER_TAG=r0.1.97.0 \
    cargo risczero build --manifest-path lc/guest/Cargo.toml
```

On a local LEZ devnet (Bedrock, the sequencer and the indexer, all pinned to `v0.4.0-rc1`):

```bash
./scripts/up.sh      # start the devnet
./scripts/lc.sh      # the whole flow on recorded mainnet, ending in PASS
./scripts/down.sh    # stop it, keeping its state; ./scripts/reset.sh wipes it
```

By default every proof is faked (`DEV_MODE=1`): the run takes minutes and the fees are the
same. `DEV_MODE=0` for both scripts proves for real, about half an hour per update on a laptop.

## Layout

| Path | What |
|---|---|
| `lc/types/` | the program's messages, state and checks: `core` (the verifier), `statement` (attestations), `mpt` (Merkle-Patricia proofs), `history` (older blocks), `oracle` (Helios, tests only) |
| `lc/guest/` | the `eth_lc` program, with RISC Zero's accelerated `blst`, SHA-256 and Keccak |
| `src/` | the operator and consumer client used on the devnet, and reading recorded chain data |
| `tests/` | the native tests, and the recorded mainnet in `tests/data/` |
| `scripts/`, `configs/` | the devnet, and the end-to-end run |
| `rfp-021/` | the proposal, the attestation design, results and logs, LEZ's code paths and costs |

## Built on

`blst` (BLS12-381, audited by NCC Group for the Ethereum Foundation), Helios `consensus-core`
(the verifier's reference in the tests), RISC Zero, alloy, and LEZ `v0.4.0-rc1`.

## Licence

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE).
