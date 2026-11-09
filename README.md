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

## Built on Helios

The light-client program, now being built, verifies updates with **Helios `consensus-core`**
(a16z/helios, MIT), the consensus logic of the Helios Ethereum light client, compiled into the
program's image for RISC Zero's zkVM, which LEZ runs on.

**Why Helios: RFP-021's R.6.** The RFP asks proposals to integrate mature, audited
implementations of the light-client protocol rather than write their own, and to say what is
reused, what is rewritten, and what that leaves for the audit.

- Helios's consensus logic is the audited one in the RISC Zero ecosystem: zkSecurity audited it
  inside `r0vm-helios`, a RISC Zero guest built on Helios `consensus-core` 0.8.3.
- `r0vm-helios` itself isn't usable here. It stops at the Electra fork, so it can't verify
  today's mainnet (Fulu); it targets risc0 2.0, where LEZ uses 3.0.5; and it proves for a
  Solidity verifier. We use current Helios, which covers Fulu and Gloas, with the same RISC Zero
  patch set `r0vm-helios` established: accelerated `bls12_381` and `sha2`.
- **Reused:** Helios's consensus checks; `alloy-trie` and `alloy-rlp` for Merkle-Patricia proofs
  and headers.
- **Ours:** the LEZ actor and its trust boundary, the compact store (committee roots, not keys),
  the 2/3 quorum enforced as an error, record accounts, statements and predicates, history
  proofs, and the operator and consumer clients.
- **For the audit:** zkSecurity's audit covers Helios's consensus logic as of 0.8.3. What it
  doesn't cover goes in our audit: Helios's changes since (Fulu, Gloas), its use inside a LEZ
  actor, and all of our own code.

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
./scripts/down.sh     # stop, keep state
./scripts/reset.sh    # stop and wipe back to genesis
```

`DEV_MODE=0`, for `up.sh` too, proves for real; the default fakes every proof. Every charge is
printed as a `FEE` line and kept in `.devnet/logs/fees.log`. Stop the devnet when you're done:
idle, its Bedrock node keeps several cores busy.

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | deploying a program, following private state, fee measurement |
| `src/bin/hello_world.rs` | the hello-world client |
| `guest/` | the hello-world program. Its own workspace, RISC-V only |
| `configs/` | devnet configs for Bedrock, the sequencer, the indexer and the wallet, from LEZ `v0.4.0-rc1` |
| `scripts/` | the devnet lifecycle, and helpers such as `header_rlp.py` |
| `rfp-021/` | the proposal |
| `.devnet/` | everything the devnet writes, and its binaries. Gitignored |

## Licence

MIT OR Apache-2.0.
