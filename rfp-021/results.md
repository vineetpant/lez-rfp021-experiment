# RFP-021 results

Every number here is **measured** (where, when) or **derived** (arithmetic shown). The methods
and the conditions for a timed run are in [bench-plan.md](bench-plan.md). Logs are in
[results/](results/).

## Cycles per action

Each message of the program, run in RISC Zero's executor at LEZ's metering, on the recorded
mainnet segment in `tests/data/mainnet-1904` (`tests/cost.rs`). measured, 9 Nov 2026.

| Action | Cycles | Message | Budget |
|---|---:|---:|---|
| Update, committee handoff (private) | 12.53M | — | proven; the fee is flat |
| Update, finality (private) | 11.39M | — | proven; the fee is flat |
| Init (public) | 1.36M | — | 10M per transaction |
| Attestation, storage slot (public) | 301,587 | 7,121 B | 2M default gas limit |
| Attestation, account balance (public) | 179,815 | 4,045 B | 2M default gas limit |
| Attestation, receipt log (public) | 83,003 | 1,761 B | 2M default gas limit |

The attestation rows are from the typed-predicate program (10 Nov 2026); the update and Init
rows didn't change with it.

For reference, the same update through Helios `consensus-core` in the program is 39.5M cycles,
about 34M of it in its BLS library (proposal section 11).

**The verifier is held to Helios** by the differential tests (`tests/oracle.rs`,
`tests/adversarial.rs`): on the recorded segment both accept every update and reach the same store
at every step; every adversarial case is refused by both; the verifier's SSZ roots equal Helios's
for every committee and header. 30 native tests, all passing.

## Node run: real proofs on a LEZ v0.4 devnet

- **Node:** one Bedrock node, the sequencer and the indexer, LEZ `v0.4.0-rc1`, reset to genesis
  before the run.
- **Data:** live Ethereum mainnet: a checkpoint at slot 15616576, its period's handoff update,
  then a finality update.
- **Proofs:** real (`DEV_MODE=0`); the sequencer verified each private proof.
- **Machine:** the reference laptop (M3 Pro, 11 cores, 18 GB, CPU, default segment size). Chrome
  and Brave were open, and the devnet's Bedrock node took 1.5–2 cores; the update times match a
  first run under the same program within 2 minutes, so they are reported, with that condition.
- **Fees:** from balances (v0.4 has no receipts), at the base-fee floor of 8. A private step's
  net charge is its reserve less the refund, which returns to the fee payer's blinded alias.

Log: [results/node-2026-11-09/run.log](results/node-2026-11-09/run.log), program image
`5031c0b1…`. measured, 9 Nov 2026.

**Private steps** (proven on the laptop):

| Step | Proving + submit | Peak memory | Net charge | Public cycles | Block |
|---|---:|---:|---:|---:|---|
| Update 1: committee handoff | **2,199.6 s (36.7 min)** | 8.4 GB | 5,333,856 | 32,905 | 222 |
| Update 2: finality, writes a new record account | **2,137.1 s (35.6 min)** | 8.7 GB | 5,526,600 | 56,998 | 365 |
| Attestation, WETH storage slot, at the record | 873.2 s (14.6 min) | 7.3 GB | 5,232,864 | 20,281 | 425 |
| Attestation, 1,000 blocks back through EIP-2935 | 911.7 s (15.2 min) | 7.7 GB | 5,232,864 | 20,281 | 487 |
| Attestation, 3 blocks back through a parent-hash chain | 853.2 s (14.2 min) | 7.8 GB | 5,232,864 | 20,281 | 545 |

The private fee is (409,764 + public cycles) × 8 + 224,063 × 8 (LEZ `PRIVATE_VERIFY_GAS`,
`PRIVATE_GAS_STOR`), so it is nearly flat. Reaching an older block privately costs nothing
extra: the history proof is checked inside the consumer's proof, and what is published is the
same `CheckRecord`.

**Public steps** (run by the sequencer):

| Step | Exec gas | Bytes | Charge | Block |
|---|---:|---:|---:|---|
| Deploy | — | — | 5,429,088 | 58–74 |
| Init, with the checkpoint's record | 1,372,713 | 26,579 | 11,194,336 | 75 |
| Attestation at the record | 301,115 | 7,514 | **2,469,032** | 366 |
| Attestation, 1,000 blocks back, EIP-2935 | 549,177 | 13,863 | **4,504,320** | 426 |
| Attestation, 3 blocks back, parent-hash chain | 402,118 | 10,131 | **3,297,992** | 488 |
| Refused: tampered proof, false claim | 2,000,000 (the limit) | 7,514 | 16,060,112 each | 546, 547 |
| Refused: forged history header | 2,000,000 (the limit) | 13,863 | 16,110,904 | 548 |

**Outcomes.** Every honest step was accepted (read from each block's `reverted` list). Every
broken one was refused: on the public path the client's dry run named the refusal first
(`InvalidInclusionProof`, `ConditionUnsatisfied`, `InvalidExecutionProof`), and sent anyway as a
negative test, each reverted; on the private path each was unprovable, so nothing was sent; an
attestation against a slot with no record was unprovable too.

**What each private transaction publishes.** After every private step the client listed every
published field and searched the whole transaction, proof included, for the private accounts the
wallet owns, the payer, the queried address and slot, the proof leaves, and for history
attestations the target's header and history slot. Every search passed. What is published: the
boundary message (`Advance` to the instance, or `CheckRecord` to the record account) and the fee
reserve, each from a one-time alias, with the refund to the payer's alias.

An earlier run of the same verifier ([first-run.log](results/node-2026-11-09/first-run.log))
proved the two updates in 2,267.5 s and 2,116.2 s; its program hashed Keccak in software, which
put a public attestation over the 2M gas limit, so the run above is on the program that routes
Keccak through RISC Zero's accelerator.

## Derived

**Tracking cost (P.3).** Two updates per sync-committee period (27.3 h), 642 a year. Per update:
466,762 exec gas (409,764 + 56,998), 224,063 storage gas, 5,526,600 at the floor.

| | Per update | Per period (2) | Per year (642) |
|---|---:|---:|---:|
| Exec gas | 466,762 | 933,524 | 3.00 × 10⁸ |
| Charge at the floor | 5,526,600 | 11,053,200 | 3.55 × 10⁹ |
| Share of the period's exec capacity (9,830 blocks × 10M) | — | 0.001% | — |

Per attestation, at the daily average of 1.76 updates:

| Attestations per day | Exec gas per attestation | Storage gas per attestation |
|---:|---:|---:|
| 10 | 82,048 | 39,386 |
| 100 | 8,205 | 3,939 |
| 1,000 | 820 | 394 |

**Latency (P.5).** An event in a block before the latest record (up to 8,191 blocks before it)
needs no update: the consumer attests at once, in one block publicly or after 14–15 min of
proving privately. An event newer than the latest record waits for the next scheduled update (up
to 13.7 h) or an on-demand one: about 12.8 min of finality, up to 6.4 min for the update to
appear, 35.6–36.7 min of proving on the reference laptop, one block: **about 48–56 min**.

**Storage (P.7).** A record account holds 186 B of state; 642 a year is 119 KB, and 597 KB over 5
years. The instance's own state is constant. No action reads more than one record (P.9).

**Refusals (R.2).** A refused public attestation is charged its whole gas limit, 6.5× an
accepted one at the record; the client's dry run stops an honest client before that.
