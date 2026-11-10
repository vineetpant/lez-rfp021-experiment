# Trustless Ethereum state attestation on LEZ

The working ground for an RFP-021 proposal: a LEZ program that tracks finalized Ethereum
through the sync committee, and lets other LEZ programs attest to facts about Ethereum state,
publicly or privately, with no committee or oracle to trust. It runs on a local LEZ
`v0.4.0-rc1` node (Bedrock, the sequencer and the indexer) against live Ethereum mainnet.

The proposal is [rfp-021/proposal.md](rfp-021/proposal.md).

## The approach

- **An operator runs the light client off-chain** and submits each sync-committee update to
  LEZ. The update is verified in a **private** transaction: the operator's machine runs the
  checks and proves them, and the sequencer verifies the proof. One transaction, at a flat
  private fee.
- **The program trusts only what it verified.** It accepts a new store only from its own code,
  on its own config, on top of the store it holds, and writes each verified header once to its
  own record account.
- **Consumers attest against a record**, publicly (the sequencer checks the Merkle-Patricia
  proof) or privately (the consumer proves it, and only a check that the record is held is
  published, from a one-time alias). History proofs reach any block in the 8,191 before a
  record, so the instance needs only two updates per sync-committee period.

## The verifier: audited cryptography, held to audited logic

The program checks each update with **our verifier** (`lc/types/src/core.rs`): the consensus
specs' light-client protocol, about 300 lines, with the BLS12-381 signature checked by **`blst`**,
the library audited by NCC Group for the Ethereum Foundation, through RISC Zero's accelerated fork.

It is held to **Helios `consensus-core`** (a16z), whose consensus logic zkSecurity audited: in the
tests every recorded mainnet update goes through both and both must reach the same store, and
every attack case must be refused by both (`tests/oracle.rs`, `tests/adversarial.rs`). This is
how RFP-021's R.6, reuse audited implementations, is met; the proposal's section 11 says why the
program doesn't run Helios itself.

It is cheap on LEZ: an update is 11.4M cycles, proven privately at a flat fee; Init is 1.4M; a
public storage attestation is 0.30M, 3% of a transaction's budget.

## Status

Working end to end on a LEZ `v0.4.0-rc1` node with real proofs, against live mainnet (9 Nov
2026): Init from a live checkpoint, a committee handoff and a finality update, public and private
attestations at the record and at older blocks (EIP-2935 and a parent-hash chain), and every
broken attestation refused. 30 native tests pass, including the verifier against Helios.

| On the node, real proofs, reference laptop (M3 Pro, 18 GB, CPU) | Time | Peak memory | Charge |
|---|---:|---:|---:|
| Init (public) | one block | — | 11,194,336 |
| Update, committee handoff (proven by the operator) | 36.7 min | 8.4 GB | 5,333,856 |
| Update, finality | 35.6 min | 8.7 GB | 5,526,600 |
| Attestation, public, at the record | one block | — | 2,469,032 |
| Attestation, public, 1,000 blocks back (EIP-2935) | one block | — | 4,504,320 |
| Attestation, private (proven by the consumer), at the record or older | 14.2–15.2 min | 7.3–7.8 GB | 5,232,864 |
| A refused public attestation | one block | — | 16,060,112 (its whole gas limit) |

Details, cycles, conditions and the derived costs (tracking, latency, storage) are in
[rfp-021/results.md](rfp-021/results.md).

## Run it

Bash, on macOS arm64 or Linux, with Rust, the RISC Zero toolchain (`rzup`) and Docker (for
reproducible guest builds).

```bash
./scripts/up.sh       # builds and starts Bedrock, the sequencer and the indexer, all native
./scripts/hello.sh    # build, deploy, call publicly and privately, check the greeting
./scripts/lc.sh       # eth_lc end to end: deploy, Init, two updates, attestations, negative cases
./scripts/down.sh     # stop, keep state
./scripts/reset.sh    # stop and wipe back to genesis
```

`DEV_MODE=0`, for `up.sh` too, proves for real; the default fakes every proof. Every charge is
printed as a `FEE` line and kept in `.devnet/logs/fees.log`. Stop the devnet when you're done:
idle, its Bedrock node keeps several cores busy.

## Layout

| Path | What |
|---|---|
| `lc/types/` | the program's messages, state and checks, shared with the client and tests: `core` (the verifier), `mpt` (inclusion proofs), `statement` (claims and statements), `history` (older blocks), `oracle` (Helios, host only) |
| `lc/guest/` | the `eth_lc` program. Its own workspace, RISC-V only, with RISC Zero's accelerated `blst`, SHA-256 and Keccak |
| `src/bin/eth_lc.rs` | the operator and consumer client |
| `src/chain.rs` | Beacon API JSON, through Helios's types, into the verifier's |
| `src/lib.rs` | deploying a program, following private state, fee measurement |
| `src/bin/hello_world.rs` | the hello-world client |
| `tests/` | native tests on recorded mainnet: the oracle and adversarial suites, history, statements, cycle costs |
| `guest/` | the hello-world program. Its own workspace, RISC-V only |
| `configs/` | devnet configs for Bedrock, the sequencer, the indexer and the wallet, from LEZ `v0.4.0-rc1` |
| `scripts/` | the devnet lifecycle, and helpers such as `header_rlp.py` |
| `rfp-021/` | the proposal, results and benchmark plan |
| `.devnet/` | everything the devnet writes, and its binaries. Gitignored |

## Licence

MIT OR Apache-2.0.
