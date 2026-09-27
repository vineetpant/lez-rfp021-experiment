# Context for Claude

## What this is

A working hello world on LEZ, built as the starting point for an RFP-021 proposal and
implementation — the Logos grant for trustless Ethereum state attestation.

It's more than a toy on purpose. The point was to prove the whole toolchain works before
committing to a grant proposal, and to leave behind a devnet and a program skeleton the
attestation module can grow inside.

It works: `reset.sh` → `up.sh` → `hello.sh` from genesis ends in `PASS`, on macOS arm64 with
Docker 28.5.2, last checked 2026-09-26. Standalone too — that was checked by renaming the
`logos-execution-zone` checkout out of the way and running the whole thing again.

`README.md` explains how it all works for a human reader. This file is the project's memory:
why things are the way they are, and what comes next.

## Rules

- **Don't change code without discussing it first and getting Vineet's approval.** Say what
  you want to change and why, then wait. Applies to source, scripts, configs, Dockerfiles.
  Questions, reading code and read-only checks are fine.
- **Commit messages are one line.** No body, no bullet lists, no file inventories. If a
  change needs a paragraph to explain it, it's probably too big for one commit. Never add AI
  or assistant attribution, co-author trailers, or "generated with" lines — the history is
  Vineet's.
- **Ask before installing anything, cloning anything, or downloading dependencies.** Give a
  short plan naming what each step touches, then wait. Read-only checks don't need
  announcing. Prefer reversible moves — rename things aside rather than deleting them.
- **Verify before claiming.** "Should work" isn't a result. Everything in the README's status
  section came from a run that was actually watched.
- Comments explain why, not what.
- Nothing in this repo tracks problems in `logos-execution-zone`. Vineet handles that
  separately, from a fork. There's a reference file for it at
  `../logos-execution-zone-findings.md`, outside this repo. Don't bring it back in here.

## Commands

```bash
./scripts/up.sh       # bedrock in Docker, then the LEZ indexer and sequencer
./scripts/hello.sh    # build, deploy, call, check the greeting came back
./scripts/down.sh     # stop, keep state
./scripts/reset.sh    # stop and wipe back to genesis
docker compose up -d  # bedrock on its own
```

Scripts are bash and need to run as bash. `common.sh` uses `BASH_SOURCE`, which zsh doesn't
set, so sourcing it from a zsh prompt resolves every path one directory too high.

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | `deploy_program` — chunk the ELF, claim accounts, write the header |
| `src/bin/hello_world.rs` | the client: deploy, call, report |
| `guest/` | the on-chain program. Own workspace, RISC-V only |
| `docker/`, `docker-compose.yml` | bedrock, from a published release binary |
| `configs/` | devnet configs for sequencer, indexer, wallet |
| `scripts/` | devnet lifecycle |
| `.devnet/` | everything the devnet writes. Gitignored. |

`src/` and `guest/` are two programs for two machines — a native client and a RISC-V
contract. They can't be merged: different target, different toolchain, and the guest can't
depend on `tokio` or `wallet`. `guest/` is its own workspace so a host build doesn't drag
RISC-V code into the graph. It builds inside a pinned container because a program's id is the
hash of its ELF, so the build has to be reproducible.

## Why things are the way they are

The reasoning matters more than the choice, because it tells you what would justify changing
it.

**Pinned to a git tag instead of a sibling checkout.** Crates come from
`logos-execution-zone` at `tag = "v0.3.0-rc1"`, and the service binaries are
`cargo install --git … --tag … --root .devnet`. They say outright that they don't offer
backward compatibility yet, so tracking a branch would break things quietly. To bump: change
`LEZ_TAG` in `scripts/common.sh` and the three git deps in `Cargo.toml` together. There's a
commented-out `[patch]` block in `Cargo.toml` for working against a local checkout.

**Bedrock comes from a published release.** `logos-execution-zone` has a resolver that builds
bedrock from the exact `logos-blockchain` revision its `Cargo.lock` names. That's stricter,
and right if you need to reproduce a specific LEZ build, but it means compiling the node from
source whenever no release matches that commit — several minutes, it needs the checkout, and
here it got OOM-killed inside Docker's 9.4 GB limit. LEZ `v0.3.0-rc1` pins `c4c86be`
(2026-09-24), one commit newer than release `0.3.0-rc.4` (`39916dc`, 2026-09-23), and that
commit touches the zone SDK. Vineet chose the latest release anyway; it was tested and it
works. If block acceptance or the channel ever misbehaves after a LEZ bump, check that gap
first — `just resolve-bedrock-node` in a checkout builds the exact revision.

**Two version pins in two places, deliberately.** `LEZ_TAG` in `scripts/common.sh`,
`BEDROCK_VERSION` in `docker-compose.yml`. Different repos, different release cadences.
Collapsing them into one variable would imply a coupling that isn't there.

**We own `deploy_program`.** It's 25 lines — chunk the ELF, claim the accounts, write the
header — so it lives here rather than reaching into someone else's checkout at build time.

**Fees go through `send_pub_tx_paid_by`.** An ordinary program call is charged, so the message
needs a named payer, that payer's nonce, and its signature. The wallet's higher-level call
assembles all of it. Building the message directly means reproducing the lot, and a message
with no fee comes back as `Incorrect fee`.

**`RISC0_DEV_MODE` is never exported globally.** risc0's build scripts hash the environment,
so exporting it invalidates them and rebuilds the guest on every cargo call. `common.sh`
defines `DEV_MODE_ENV` and passes it only to processes that actually run programs.

**All devnet state lives under `.devnet/`.** An earlier version scattered RocksDB directories
and a signing key into Vineet's `logos-execution-zone` checkout. Now a reset is one directory
removal and nothing leaks.

**Scripts fail loudly, and that's not decoration.** Three bugs during this build were silent
failures, and all three looked like success:

1. A `grep -q` swallowed `Error: Failed to find leader`, so a transient that would have been
   obvious looked like a five-minute hang with no explanation.
2. `set -e` with `pipefail` killed a script when a `grep` after a fixed `sleep 5` matched
   nothing — the read had simply happened before the block executed.
3. A leftover sequencer from an older layout kept ports 3040 and 8779 and answered the health
   check for a devnet that wasn't running, producing a false `PASS` that got reported as
   proof of standalone operation. `down.sh`'s `pkill` pattern had been narrowed to
   `.devnet/bin` paths and no longer matched it.

So: retry with a deadline and print what failed, poll for state instead of sleeping a guess,
and refuse to start when something already holds 18080, 3040 or 8779. Keep that. It's exactly
the failure class the RFP-021 submitter lives in — submit, wait for inclusion, read derived
state — where a stale answer is worse than an error.

## Known rough edges

- **`down.sh` then `up.sh` doesn't work.** `docker compose down` removes the bedrock
  container, and bedrock's chain state lives inside it, so the chain comes back empty while
  `.devnet/state/sequencer` still holds a checkpoint for the old channel. The sequencer
  refuses to resume and says so. `reset.sh` → `up.sh` is the working cycle. Proposed fix,
  awaiting Vineet's approval: have `down.sh` use `docker compose stop` so the container
  survives, leave `down` for `reset.sh`, and have `up.sh` catch the mismatch and point at
  `reset.sh`.
- `wallet account get --scope all` says the account is uninitialised even when its shard
  holds data. Reads have to name the program.
- Guest builds need `RISC0_DOCKER_CONTAINER_TAG=r0.1.91.1`. The default image's rustc is
  older than this dependency tree wants.
- `SUPPRESS_VERBOSE_PRINTS=1`, or the wallet prints about 1.2 MB of raw transaction bytes per
  call.
- A submitted transaction is only in the mempool. Its shard can't be read until the block
  carrying it executes.
- The sequencer and indexer both put their RocksDB in the working directory, so each needs
  its own.
- In a `logos-execution-zone` checkout, `cargo build` from the repo root can fail on
  `risc0-sys v1.5.0`. Use `-p`, or build from the crate directory.

## Timings, for comparison later

This project 4m39s cold; the LEZ service binaries about 8 minutes; the bedrock image about a
minute; devnet healthy 10–20 seconds after start; deploy plus call about 25 seconds across
five blocks. The guest itself runs in 1.6–2.2 ms in dev mode, which is the number to measure
anything heavier against.

## Background

The RFP-021 text is in `logos-co/rfp`, on branch `rfp-021-wrapped-erc20` — the branch name is
misleading — as `RFPs/RFP-021-ethereum-state-attestation.md`. It's **not on master**, still a
draft, so re-check it before citing anything.

Vineet's earlier analysis lives in two claude.ai artifacts, "LEZ and RFP-021 — working
reference" and "RFP-021: Ethereum State Attestation for LEZ". The first cites RFP commit
`ceac7e4` (2026-08-27). It's still accurate except on supportability: docs moved in-repo, and
the team now has to build its own module catalog from `logos-modules-release-base` and release
through `logos-modules-release-action`. The architecture and risk analysis hold.

## What's next

Roughly in order of how much risk each one removes.

**1. Cycle-count BLS12-381.** The RFP's own central question is whether in-program aggregate
signature verification fits LEZ's per-transaction cycle budget at the cadence sync-committee
tracking needs. A throwaway guest doing one aggregate verify over 512 public keys, with the
cycles counted, answers it either way — and if it doesn't fit, that measurement is itself a
deliverable, with the specific component named. The guest pipeline here is ready for it; it
goes in `guest/src/bin/`. One thing to settle first: which BLS library. `blst`'s assembly
won't cross-compile to `riscv32im`, and a pure-Rust arkworks path will compile but runs
slower, which skews the number. That choice decides whether the measurement means anything.

**2. Measure the private path.** The RFP wants wall-clock and peak memory on a mid-range
laptop and on the lowest spec the team declares. Hashing is cheap per operation but repeats
all the way down the Merkle path, and nobody has measured whether the total is interactive.
That answer decides whether the primitive is usable privately at all.

**3. Check whether `r0vm-helios` can actually be reused.** The biggest schedule risk in the
proposal. Does its consensus verification — sync-committee tracking, handoffs, BLS verify, SSZ
branches — come apart cleanly from its EVM-target layer, which LEZ throws away entirely? A
clean split means fork and retarget. Entangled means extraction, and the estimate moves a lot.
The RFP budgets this evaluation as a deliverable; reuse is expected to be assessed seriously,
not mandated. This is reading, not building.

**4. Design the instance model.** The checkpoint, chain config and tracked consensus state are
all accounts, because programs are stateless. Worth knowing from this work:
`program-loader deploy --immutable` is documented as self-declared and explicitly *not*
protocol-enforced, while the RFP wants immutability to be structural. So the requirement is
met by not exposing a mutating instruction, not by setting a flag — and the proposal should
say that outright.

**5. Then the real work:** inclusion verification (MPT for accounts and storage, SSZ for
beacon state, the receipts trie for logs), the predicate interface, the statement identifier
consumers key a nullifier on, and the permissionless update submitter with its varied defer
interval. Anything touching Ethereum has to go through the Logos Ethereum core module — a hard
platform dependency that shapes the whole off-chain half.

Constraints to price in: Tier M, 8–12 weeks excluding audit lead time, MIT and Apache-2.0;
docs in repo; your own module catalog from `logos-modules-release-base` released via
`logos-modules-release-action`; and an independent third-party audit of the consensus and
inclusion logic with the report published.

## Not done, on purpose

- **No private-path demo.** `hello_world` runs publicly. The same bytecode on a private
  account would run locally and produce a RISC0 proof — 30 seconds to 4 minutes. Worth adding,
  since RFP-021 has a hard privacy floor.
- **One sequencer, one validator.** No committee or multi-sequencer setup.
