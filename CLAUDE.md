# Context for Claude

## What this is

The working ground for an RFP-021 proposal (`rfp-021/proposal.md`): `eth_lc`, a LEZ program
that tracks finalized Ethereum through the sync committee on Helios's consensus checks, and lets
other LEZ programs attest to Ethereum state publicly or privately. It runs on a local LEZ
`v0.4.0-rc1` node (Bedrock, sequencer, indexer) against live mainnet, with real proofs. A
hello-world program on the same devnet is the toolchain's smoke test.

`README.md` explains it for a human reader. This file is the project's memory: why things are
the way they are.

## Rules

- **Don't change code without discussing it first and getting Vineet's approval.**
- **Commit messages are one line.** No body, no AI attribution, no co-author lines.
- **Ask before installing, cloning or downloading anything.**
- **Verify before claiming.** Every number in the proposal and results comes from a run that
  was watched; a figure not yet run is marked ⟨measure⟩.
- **No personal names in the repo**, and the repo doesn't compare itself with other proposals.
- Comments explain why, not what.

## Commands

```bash
./scripts/up.sh       # Bedrock, the indexer and the sequencer, all native
./scripts/hello.sh    # hello world: deploy, call publicly and privately; funds a private fee payer
./scripts/lc.sh       # eth_lc end to end: deploy, Init, two updates, attestations, negative cases
./scripts/down.sh     # stop, keep state
./scripts/reset.sh    # stop and wipe back to genesis
cargo test --release  # native tests: honest chain, adversarial suite, history, statements
```

`DEV_MODE=0`, for `up.sh` too, proves for real. `LC_FROM=update2|attest|history` resumes
`lc.sh` on the instance in `.devnet/lc-last`. Scripts are bash: `common.sh` uses `BASH_SOURCE`.

## Layout

| Path | What |
|---|---|
| `lc/types/` | messages, state and checks, shared by program, client and tests: `verify` (Helios), `mpt` (alloy-trie), `statement`, `history` |
| `lc/guest/` | the `eth_lc` program. Its own workspace, RISC-V only, with RISC Zero's accelerated forks patched in |
| `src/bin/eth_lc.rs` | the operator and consumer client |
| `src/chain.rs` | Beacon API JSON into Helios's types; `eth_getProof` parsing |
| `src/lib.rs` | deploying, following private state, fee measurement |
| `guest/`, `src/bin/hello_world.rs` | hello world |
| `tests/` | native tests; `tests/data/` holds recorded mainnet |
| `rfp-021/` | the proposal, results and benchmark plan |

## Why things are the way they are

**Helios `consensus-core` is the consensus logic** (RFP R.6: integrate audited
implementations). zkSecurity audited it, at 0.8.3, inside `r0vm-helios`. We use current Helios
(pinned by commit in `lc/types/Cargo.toml`) because 0.8.3 stops at Electra and mainnet runs Fulu.
The audit doesn't cover Helios's changes since 0.8.3; the proposal says so.

**The store keeps committee roots, not keys.** Helios's store holds both 512-key committees.
`verify::update` takes the keys from the submitter, checks them against the stored roots, and
builds Helios's store for one check. It also refuses below 2/3 participation as an error:
Helios applies nothing below 2/3 but doesn't refuse it.

**The guest needs RISC Zero's accelerated forks and Helios's own patch.** `bls12_381` and `sha2`
(both 0.9 and 0.10 are in the tree) from RISC Zero, and `ethereum_hashing` from the fork Helios
patches in, which hashes with `sha2` instead of `ring` (ring doesn't build for the zkVM).

**The guest builds with the rustc 1.97 builder image** (`LC_BUILDER_TAG`, `r0.1.97.0`), under
amd64 emulation, because alloy 2.5 needs rustc 1.94 and that image has no arm64 build. Hello world
still builds with `GUEST_BUILDER_TAG`.

**Fees are measured from balances.** v0.4 exposes no receipts. A private step's refund goes to
the fee payer's blinded alias, the wallet's default, and arrives as a pending credit; the client
reads it from there. After every private step a `PRIVACY` line must say `pass`.

**Init gets a 10M gas limit, nothing else does.** Init checks the bootstrap publicly. A refused
public transaction is charged its whole limit, so attestations keep the default 2M and the
client dry-runs them first.

## Traps

- **Stop the devnet when a run is done.** Idle, its Bedrock node keeps cores busy, and a
  long-lived one slows proofs.
- **Never edit `lc.sh` while it runs.** Bash reads it as it goes.
- **A long-lived devnet stalls** after about 1,350 LEZ blocks when the sequencer's Bedrock
  funding key runs out ("Failed to fund channel transaction"). Top it up from the node's SDP key:
  `POST localhost:18080/wallet/transactions/transfer-funds` with the keys in
  `configs/bedrock/node-config.yaml`.
- **Timed runs need a quiet machine**: a fresh devnet, no second browser, the Mac kept awake
  (`caffeinate -dims`). See `rfp-021/bench-plan.md`.
- **A fresh cargo target dir can fail on `rust-rapidsnark`**, a LEZ dependency that downloads a
  library at build time; a 504 from its server is transient. Reusing a built target dir avoids it.
- **rust-analyzer holds the target-dir lock** while it builds; a second cargo waits silently.
- macOS's `seq` prints large numbers as `2.6e+07`; count with a bash loop.
- RPC nodes shorten quantities in `eth_getProof` (`0x4a8`); the parser left-pads them.
