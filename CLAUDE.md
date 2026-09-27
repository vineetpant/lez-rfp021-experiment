# Context for Claude

## What this repo is

A working hello world on LEZ (Logos Execution Zone), built to serve as the **foundation for
an RFP-021 proposal and implementation** — Logos' open grant for trustless Ethereum state
attestation on LEZ.

It is deliberately more than a toy. The goal was to prove the whole toolchain end to end
before committing to a grant proposal, and to leave behind a devnet and a program skeleton
that the attestation module can grow inside.

**Status: verified green.** `reset.sh` → `up.sh` → `hello.sh` from genesis ends in `PASS`
(macOS arm64, Docker 28.5.2, 2026-09-26). Standalone operation proven by renaming the
`logos-execution-zone` checkout aside and repeating the cycle.

Read `README.md` for the user-facing explanation of how it all works — architecture,
the two-programs model, the upstream defect table. This file is the project's memory:
decisions, rationale, and what comes next.

## Commands

```bash
./scripts/up.sh       # bedrock (Docker) + LEZ indexer and sequencer
./scripts/hello.sh    # build, deploy, call, assert the greeting round-trips
./scripts/down.sh     # stop, keep state
./scripts/reset.sh    # stop and wipe state, back to genesis
docker compose up -d  # bedrock alone
```

Scripts are bash and must run as bash — `scripts/common.sh` uses `BASH_SOURCE`, which zsh
does not set, so sourcing it from zsh resolves every path one directory too high.

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | `deploy_program`: chunk the ELF, claim accounts, write the header |
| `src/bin/hello_world.rs` | host-side runner: deploy, call, report |
| `guest/` | **the on-chain program.** Own workspace, `riscv32im-risc0-zkvm-elf` only |
| `docker/`, `docker-compose.yml` | bedrock from a published release binary |
| `configs/` | vendored sequencer / indexer / wallet devnet configs |
| `scripts/` | devnet lifecycle |
| `.devnet/` | everything the devnet writes. Gitignored. |

`src/` and `guest/` are two programs for two machines: native client vs RISC-V contract.
They cannot merge — different target, different toolchain, and the guest cannot depend on
`tokio` or `wallet`. `guest/` is a separate workspace so a host build does not pull RISC-V
code into its graph.

## Decisions and why

Each of these was a real fork in the road. The rationale matters more than the choice,
because it says what would justify revisiting it.

**Standalone via git tags, not a sibling checkout.** Crates come from
`logos-execution-zone` pinned to `tag = "v0.3.0-rc1"`; service binaries are
`cargo install --git … --tag … --root .devnet`. Upstream states it offers no backward
compatibility yet, so a floating branch would rot silently. *Revisit when:* bumping to a
newer LEZ tag — change `LEZ_TAG` in `scripts/common.sh` and the three git deps in
`Cargo.toml` together. `Cargo.toml` has a commented `[patch]` block for local development.

**Bedrock from a published GitHub release, not upstream's resolver.** Upstream resolves the
exact `logos-blockchain` revision LEZ's `Cargo.lock` pins and source-builds when no release
matches — 8 minutes, needs the checkout, and it was OOM-killed (exit 137) at Docker's 9.4 GB
limit. LEZ `v0.3.0-rc1` pins `c4c86be` (2026-09-24), one commit *newer* than the newest
release `0.3.0-rc.4` (`39916dc`, 2026-09-23), and that commit touches the zone SDK. Tested
anyway on the user's instruction that latest is fine: it works. *Revisit when:* the
sequencer starts rejecting blocks or the channel misbehaves after a LEZ bump — that
mismatch is the first suspect, and `just resolve-bedrock-node` in the upstream checkout
builds the exact revision.

**Two version pins in two places, on purpose.** `LEZ_TAG` in `scripts/common.sh`,
`BEDROCK_VERSION` in `docker-compose.yml`. They come from different repos with independent
release cadences; collapsing them into one variable would imply a coupling that does not
exist.

**Our own `deploy_program` rather than depending on upstream's example crate.** It is 25
lines, and the alternative was copying a loose `.rs` into the user's checkout at build time,
which an earlier iteration did and which was rightly criticised.

**`send_pub_tx_paid_by`, not a hand-rolled fee.** Upstream's runners build messages with the
fee-exempt `Message::try_new` and are all rejected with `InvalidParams: "Incorrect fee"`.
The wallet's higher-level call assembles selectors, nonces, fee declaration and witness set
correctly; reimplementing that by hand would duplicate logic that already exists and drifts.

**`RISC0_DEV_MODE` is never exported globally.** It is part of the environment risc0's build
scripts hash, so exporting it invalidates them and forces guest rebuilds on every cargo
invocation. `common.sh` defines `DEV_MODE_ENV` and passes it only to processes that execute
programs.

**All devnet state under `.devnet/`.** Earlier versions wrote RocksDB directories and a
signing key into the user's `logos-execution-zone` checkout. Now a reset is one directory
removal and nothing leaks.

**Scripts fail loudly.** Three bugs during development were *silent* failures, and all three
looked like success:

1. A `grep -q` swallowed `Error: Failed to find leader`, so a diagnosable transient looked
   like an inexplicable five-minute hang.
2. `set -e` plus `pipefail` killed the script when a `grep` after a fixed `sleep 5` matched
   nothing — the read simply happened before the block executed.
3. A leftover sequencer from an earlier layout kept ports 3040/8779 and **answered the
   health check for a devnet that was not running**, producing a false `PASS` that was
   briefly reported as proof of standalone operation. `down.sh`'s `pkill` pattern had been
   narrowed to `.devnet/bin` paths and no longer matched it.

Hence: retry with a deadline and print the failing output; poll for state rather than
sleeping a guessed interval; and `up.sh` refuses to start when something already holds
18080, 3040 or 8779, naming the process. Keep that property — it is exactly the failure
class the RFP-021 submitter lives in (submit, await inclusion, read derived state), where a
stale answer is worse than an error.

## Traps that cost real time

`README.md` has the full ten-defect table. The ones most likely to bite again:

- `wallet account get --scope all` reports `Account is Uninitialized` **even when the shard
  holds data**. Reads must name the program: `--scope <program-account-id>`.
- `SUPPRESS_VERBOSE_PRINTS=1` or the wallet prints ~1.2 MB of raw transaction bytes per call.
- Guest builds need `RISC0_DOCKER_CONTAINER_TAG=r0.1.91.1`; the default image's rustc is too
  old for the dependency tree.
- A submitted transaction is only in the mempool. Its shard is unreadable until the carrying
  block executes — poll, never sleep a guessed interval.
- Indexer and sequencer both default RocksDB to `./rocksdb-<channel-id>` relative to the
  working directory, so each needs its own directory.
- `cargo build` from the upstream repo root fails on `risc0-sys v1.5.0`; use `-p` or build
  from the crate directory. Not diagnosed further — it was a side quest.

## Working conventions here

- **No code changes without discussing them first and getting approval.** Propose what you
  intend to change and why, then wait. This applies to source, scripts, configs and
  Dockerfiles — not to answering questions, reading code, or running read-only checks.
- **Commit messages are a single line.** No body, no bullet lists, no file inventories, no
  marketing. If a change seems to need a paragraph to justify it, the change is probably too
  big for one commit. **Never add AI or assistant attribution**, co-author trailers, or
  "generated with" lines — the history reads as the author's own work.
- **Ask before installing toolchains, cloning repos, or pulling anything onto the machine.**
  Give a short plan naming what each step touches first. Read-only checks are fine
  unannounced. Prefer reversible moves — rename to `.broken-partial`, do not `rm -rf`.
- **Verify before claiming.** "Should work" is not a result. Every claim in `README.md`'s
  Status section came from an observed run, and the standalone claim came from actually
  hiding the checkout.
- Comments explain *why*, not what. The upstream-defect workarounds each carry the reason
  they exist, so nobody removes them as noise.
- Prefer upstream's own pins (its `Justfile`, its `rust-toolchain.toml`) over inventing
  versions.

## Where the background research lives

- **RFP-021 text:** `logos-co/rfp`, branch `rfp-021-wrapped-erc20` (misleading name), file
  `RFPs/RFP-021-ethereum-state-attestation.md`. **Not on master** — still a draft, re-check
  before citing.
- The user's prior analysis is in two claude.ai artifacts: "LEZ and RFP-021 — working
  reference" and "RFP-021: Ethereum State Attestation for LEZ". The first cites RFP commit
  `ceac7e4` (2026-08-27) and is stale only in Supportability: docs moved in-repo, and the
  team must now build its own module catalog from `logos-modules-release-base` and release
  via `logos-modules-release-action`. Architecture and risk analysis still hold.

## Next moves — the path to RFP-021

Ordered by what de-risks the most per unit of effort.

**1. BLS12-381 cycle spike.** The RFP's own central feasibility question is whether
in-program aggregate signature verification fits LEZ's per-transaction cycle budget at the
cadence sync-committee tracking requires. A throwaway guest doing one aggregate verify over
512 public keys, cycle-counted, answers it either way — and if it does *not* fit, that
measurement is itself a deliverable, with the specific component named. This repo now has a
working guest pipeline to drop that program into; it belongs in `guest/src/bin/`. Baseline
for comparison: `hello_world` executes in 1.6–2.2 ms in dev mode.

**2. Measure the private path too.** The RFP wants wall-clock and peak memory on a mid-range
laptop and on the lowest declared spec. Hashing is cheap per operation but repeats down the
whole Merkle path; whether the total is interactive is unmeasured and determines whether the
primitive is usable privately at all.

**3. Assess `r0vm-helios` reuse.** The single biggest schedule risk. Check whether its
consensus verification (sync-committee tracking, handoffs, BLS verify, SSZ branches)
separates cleanly from its EVM-target layer (proof wrapping, Solidity verifier, gas
accounting), which LEZ discards entirely. Cleanly split means fork and retarget; entangled
means extraction, and the estimate changes a lot. The RFP scopes this evaluation as a
budgeted deliverable — reuse is expected to be assessed seriously, not mandated.

**4. Design the instance model.** Checkpoint, chain config and tracked consensus state are
all *accounts*, because programs are stateless. Note a finding from this work:
`program-loader deploy --immutable` is documented as "self-declares as immutable **(not
protocol-enforced)**", while the RFP demands immutability be structural. So the requirement
is met by **not exposing a mutating instruction**, not by a flag — and the proposal should
say so explicitly.

**5. Then the substance:** inclusion verification (MPT for accounts and storage, SSZ for
beacon state, receipts trie for logs), the predicate interface, the statement identifier
consumers key a nullifier on, and the permissionless update submitter with its varied defer
interval. Ethereum-side data must go through the **Logos Ethereum core module** — a hard
platform dependency that shapes the off-chain half.

### Constraints to price into the proposal

- Tier M, 8–12 weeks excluding audit lead time, MIT and Apache-2.0.
- Documentation **in repo** (changed from doc packets).
- Publish modules in **your own catalog** built from `logos-modules-release-base`, released
  via `logos-modules-release-action`. Both are newer than the user's notes.
- Independent third-party audit of consensus and inclusion logic, report published.

## Deliberately not done

- **No `run_hello_world_paid.rs` in the upstream checkout anymore** — that approach was
  replaced by this standalone crate. An earlier session left such a file in
  `logos-execution-zone/examples/program_deployment/src/bin/`; it is harmless but is not
  this repo's.
- **No upstream PRs filed, and they are out of scope here.** Ten findings, eight blocking,
  one a genuine code bug (`Message::try_new` in every runner) — tabulated in `README.md`.
  Vineet will raise these with upstream himself, separately: they belong neither to this
  repo nor to the RFP-021 work. Do not start on them or offer again.
- **No private-path demo.** `hello_world` runs publicly. The identical bytecode on a private
  account would execute locally and produce a RISC0 proof (upstream: 30 s to 4 min). Worth
  adding, since RFP-021's privacy floor is a hard requirement.
- **Multi-sequencer / committee setup.** The devnet is one sequencer and one validator.
