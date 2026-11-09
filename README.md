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

## The verifier, and Helios as its reference

The program checks each update with **our verifier** (`lc/types/src/core.rs`): the consensus
specs' light-client protocol, about 300 lines, with BLS12-381 through **`blst`** and RISC Zero's
accelerator. We wrote the protocol steps (period checks, finality and committee branches, the
signing domain, the 2/3 quorum, the store transitions); the cryptography is `blst`'s.

**Helios `consensus-core`** (a16z/helios, MIT) is its reference. RFP-021's R.6 asks proposals to
reuse mature, audited light-client implementations, and Helios's consensus logic is the one
zkSecurity audited inside `r0vm-helios`, on the zkVM LEZ uses. We evaluated it inside the program
first, and measured:

| One update, in the program | Cycles |
|---|---:|
| Helios `consensus-core` | 39.5M, about 34M of it in its BLS library (`bls12_381`) |
| Our verifier, on `blst` | **11.4M** |

The protocol logic is the same either way; the difference is the BLS library, which Helios
doesn't let us swap. So Helios checks our verifier instead, in the tests (`tests/oracle.rs`,
`tests/adversarial.rs`): every recorded mainnet update, through two committee handoffs, goes
through both, and both must reach the same store; every attack (a fabricated chain signed by a
committee we generated, a committee never handed off, a forged next committee, too few signers,
an unfinalized header, tampered roots, another chain's domain, a replay) must be refused by both.

`r0vm-helios` itself isn't usable: it stops at the Electra fork, so it can't verify today's
mainnet (Fulu), and it targets risc0 2.0 and a Solidity verifier.

## Status

- **Works:** a hello-world program on LEZ `v0.4.0-rc1`, deployed and called publicly and
  privately, with fees measured from balances, on a native local devnet.
- **In progress:** the light-client program on Helios, its tests, and its runs on the node with
  real proofs. Every figure in the proposal marked ⟨measure⟩ comes from those runs.

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
