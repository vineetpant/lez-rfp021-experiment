# LEZ RFP-021 experiment

A LEZ program, `eth_lc`, that follows finalised Ethereum through the sync committee and lets
other LEZ programs prove facts about Ethereum (a storage slot, an account, an event log) in
their own transaction, publicly or privately. It is the working prototype behind our
[RFP-021](https://github.com/logos-co/rfp) proposal, **Caisson**
(<https://claude.ai/artifact/NEstvDGmVwQC6zGXgWzpNS>), on LEZ `v0.4.0-rc1`.

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
  (public, or private: only               an ID per fact
  CheckRecord is published)
```

1. **The light-client check** is one function, `core::update` (`crates/types/src/core.rs`): the
   consensus specs' sync protocol and its store, with the BLS signature checked by `blst`.
   It is the verifying half of a light client; fetching is the operator's, and untrusted.
2. **The operator proves it privately** and submits the proof. The sequencer verifies it
   without re-running the BLS check (`Update` in `crates/program/src/bin/eth_lc.rs`).
3. **The instance advances**, and the finalised header is written once to its own account,
   derived from the instance and the execution block number (`Advance`, `WriteRecord`).
4. **A consumer sends a claim**: facts, each a Merkle-Patricia proof against a finalised header
   with conditions on its values, such as `StorageValue`, `AccountBalance` or `LogDataWord`
   (the proposal's sections 4.5–4.7).
5. **`statement::check`** verifies the proofs, evaluates the conditions and returns a statement
   with one ID per fact, from the Ethereum state alone, for the consumer's nullifier.

The types are the proposal's (section 4): `InstanceConfig`, `FinalisedHeader`, `Claim`,
`Fact`, `Condition`, `Statement`, `ProvenFact`, `Event`. The verifier is held to Helios's
`consensus-core` in the tests: on every recorded update and every attack case, both reach the
same verdict and store.

## What it costs

**Cycles per action**, run through the program in RISC Zero's executor, metered as LEZ meters
it (`cargo test --release --test cost -- --nocapture`; 11 Nov 2026):

| Action | Path | Cycles | Message |
|---|---|---:|---:|
| Update, committee handoff | private, proven by the operator | 12,554,439 | — |
| Update, finality | private, proven by the operator | 11,369,739 | — |
| Init | public | 1,359,479 | — |
| Attestation, storage slot with its account | public | 301,471 | 7,122 B |
| Attestation, account | public | 179,974 | 4,044 B |
| Attestation, receipt log | public | 83,076 | 1,760 B |
| Each extra condition | public | 1,486 | 36 B |

An update is over a public transaction's 10M cap, so it is proven privately; LEZ doesn't meter
proven work, and charges a private transaction a flat (409,764 + its public cycles) × 8 +
224,063 × 8. Public attestations fit easily in the 2M default gas limit.

**On a LEZ v0.4 devnet, with real proofs** (9 Nov 2026; Apple M3 Pro, 18 GB, CPU proving,
default segment size, base fees at the floor of 8):

| Action | Time | Peak memory | Charge |
|---|---:|---:|---:|
| Init | one block | — | 11,194,336 |
| Update, committee handoff | 36.7 min proving | 8.4 GB | 5,333,856 |
| Update, finality | 35.6 min proving | 8.7 GB | 5,526,600 |
| Attestation, public, storage slot | one block | — | 2,469,032 |
| Attestation, private, storage slot | 14.6 min proving | 7.3 GB | 5,232,864 |
| Attestation, public, 1,000 blocks back (EIP-2935) | one block | — | 4,504,320 |
| Attestation, public, refused | one block | — | 16,060,112 (its whole gas limit) |

The devnet was reset before the run and the Mac kept awake; Chrome and Brave were open, and a
first run gave update times within 2 minutes. Re-run on 11 Nov on the program with the
proposal's data model (proofs faked, fees the same): public attestation 301,509 exec gas and
7,520 B; private attestation 22,650 public cycles; an update's public part 4–9k cycles more,
for the header and handoff events. Every privacy check passed and every broken attestation was
refused, in both runs.

**Derived**, at two updates per sync-committee period (642 a year):
- tracking costs about **11.2M a period** at the base-fee floor, 3.59 × 10⁹ a year, 0.001% of
  the period's exec capacity;
- a finalised header's account holds 226 B: **145 KB a year**, 725 KB over 5 years;
- an event in a block before the latest header is attested at once; a newer one waits for an
  update, **about 48–56 min** on the reference laptop (finality, update availability, proving).

**Not measured yet.** Every attestation run on the node was a storage-slot claim. Account and
receipt-log claims are tested natively and their cycles measured above, but their transaction
size on the node and their private proving time and memory are still to measure; their private
fee is the same as the storage slot's, because the published `CheckRecord` doesn't depend on
the claim. Proving at smaller segment sizes and on a GPU is also still to measure.

## Tested against recorded mainnet

Init and the light-client updates run against recorded mainnet data in
[`tests/data/mainnet-1904`](tests/data/mainnet-1904): a checkpoint at slot 15,597,568, the
updates through two committee handoffs, a finality update, a WETH `eth_getProof` and the
block's receipts. The same data drives both:

- **the native tests**: the program's code against Helios, the attack cases, claims, history
  proofs (`tests/data/history`) and the cycle costs;
- **the devnet runs** above: deploy, Init, every update proven and submitted, public and
  private attestations, and broken attestations, each of which must be refused.

The program refuses an update more than 14 days past its store, the freshness bound that the
sequencer's clock enforces, so a devnet run on this data works until about **20 Nov 2026**. The
native tests don't depend on the clock and keep passing.

## Run it

```bash
cargo test --release                               # every native test
cargo test --release --test cost -- --nocapture    # the cycle table above
```

The cost test needs the program built first, in a pinned container (Docker, and the RISC Zero
toolchain: `curl -L https://risczero.com/install | bash && rzup install`):

```bash
DOCKER_DEFAULT_PLATFORM=linux/amd64 RISC0_DOCKER_CONTAINER_TAG=r0.1.97.0 \
    cargo risczero build --manifest-path crates/program/Cargo.toml
```

The scripts and configs for the local LEZ devnet (Bedrock, the sequencer and the indexer, all
pinned to `v0.4.0-rc1`) that produced the node figures are in
[`devnet/` at commit `79b183a`](https://github.com/vineetpant/lez-rfp021-experiment/tree/79b183a/devnet),
where they last ran end to end, ending in PASS. `git checkout 79b183a -- devnet` brings them back.

## Layout

| Path | What |
|---|---|
| `crates/types/` | the program's messages, state and checks: `core` (the verifier), `statement` (claims), `mpt` (Merkle-Patricia proofs), `history` (older blocks), `oracle` (Helios, tests only) |
| `crates/program/` | the `eth_lc` program, with RISC Zero's accelerated `blst`, SHA-256 and Keccak |
| `src/` | the operator and consumer client used on the devnet, and reading recorded chain data |
| `tests/` | the native tests, and the recorded mainnet in `tests/data/` |

## Built on

`blst` (BLS12-381, audited by NCC Group for the Ethereum Foundation), Helios `consensus-core`
(the verifier's reference in the tests), RISC Zero, alloy, and LEZ `v0.4.0-rc1`.

## Licence

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE).
