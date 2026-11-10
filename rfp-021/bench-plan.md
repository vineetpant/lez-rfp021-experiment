# Benchmark plan

What is measured, how, and under which conditions. Results go in [results.md](results.md), each
tagged **measured** (who, where) or **derived** (arithmetic shown).

## Conditions for a timed run

Proving time on a laptop depends on how much of the machine the prover gets, so a time counts
only when the run meets these:

- **A fresh devnet.** Reset it before a timed run; a long-lived Bedrock node keeps several cores
  busy and slows proofs.
- **A light desktop load.** No second browser, no other heavy process. A proof wants about 9 GB
  free on the reference laptop.
- **The Mac kept awake**, on power, with `caffeinate -dims`; idle sleep pauses the prover.
- **The conditions recorded** with the run: the apps open at the start.
- **A starved run is discarded** and repeated. Its fees and outcomes still count, because they
  don't depend on speed.

The reference laptop is an Apple M3 Pro, 11 cores, 18 GB, CPU proving, default segment size.

## What is measured

| | What | How | When |
|---|---|---|---|
| Cycles per action | Init, a handoff update, a finality update, an attestation per proof kind (storage, account, receipt log) | RISC Zero's executor at LEZ's metering (`tests/cost.rs`) | done |
| Node runs | Every action's charge, proving time, peak memory, outcome; what each private transaction publishes | `scripts/lc.sh` on a fresh v0.4 devnet, real proofs | done |
| Verifier against Helios | Same verdict and store on recorded mainnet and every adversarial case | `tests/oracle.rs`, `tests/adversarial.rs` | done |
| Lowest workable RAM | Proving at smaller segment sizes | the prover's segment limit | M0 |
| GPU proving | An update and an attestation on a GPU | a GPU machine | M0 |
| Public split | The verifier as public steps under the 10M cap; the pairing's share (P.8) | the executor | M0 |
| Catch-up | Several periods in one private transaction | a stalled devnet instance | M1 |
| Multi-operator | Racing and deferral | two operators on one instance | M3 |
| Final 0.4 | Everything again on the released LEZ 0.4 | the same scripts | M4 |
