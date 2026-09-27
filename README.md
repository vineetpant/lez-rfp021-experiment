# LEZ hello world

A working "Hola mundo!" on the [Logos Execution Zone](https://github.com/logos-blockchain/logos-execution-zone):
a Rust program compiled to RISC-V, deployed to a local devnet, executed on-chain, and read
back out of an account shard.

```bash
./scripts/up.sh       # bedrock in Docker, then the LEZ indexer and sequencer
./scripts/hello.sh    # build, deploy, call, verify -> prints PASS
./scripts/down.sh     # stop the devnet, keep state
./scripts/reset.sh    # stop and wipe state, back to genesis
```

`hello.sh` decodes the shard and asserts it equals `Hola mundo!`, so it reports a real pass
or fail rather than the mere absence of errors.

**No `logos-execution-zone` checkout required.** The crates come from git pinned to a tag,
the service binaries are installed from that same tag into `.devnet/bin`, and bedrock's
image pulls a published release binary. Verified by hiding the checkout entirely and
running the full cycle.

## Prerequisites

macOS arm64, verified 2026-09-26. Docker must be running.

```bash
brew install pkg-config
curl -L https://risczero.com/install | bash   # rzup into ~/.risc0; appends to your shell profile
rzup install                                  # cargo-risczero, r0vm, risc0 rust toolchain
```

Plus [Rust](https://rustup.rs). The pinned 1.98.1 comes from `rust-toolchain.toml`, which
rustup fetches as a directory override without touching your default toolchain.

That is the whole list. If you would rather not run `curl | bash`, download the script and
read it first — it is about 240 lines and only fetches the `rzup` binary into
`~/.risc0/bin` and adds that to your `PATH`.

First run builds a fair amount: about 5 minutes for this project, 8 for the LEZ service
binaries, 1 for bedrock's image, plus the guest. Everything after that is cached.

## What gets created

Nothing is written outside this directory. All of it is gitignored.

```
.devnet/
├── bin/              wallet, sequencer_service, indexer_service (+ bedrock_pubkey)
├── state/indexer/    RocksDB
├── state/sequencer/  RocksDB, signing key
├── wallet/           LEE_WALLET_HOME_DIR: keys, account labels
└── logs/             indexer.log, sequencer.log, health.log
```

Each service gets its own state directory because both default their RocksDB to
`./rocksdb-<channel-id>` relative to the working directory — run two from one place and the
second dies on the first's lock.

## Layout

```
├── Cargo.toml                  host crate: LEZ crates via git, tag v0.3.0-rc1
├── rust-toolchain.toml
├── docker-compose.yml          bedrock
├── docker/
│   ├── bedrock.Dockerfile      fetches a published logos-blockchain-node release
│   └── bedrock/                its config, KZG params, entrypoint
├── configs/                    vendored sequencer / indexer / wallet configs
├── src/
│   ├── lib.rs                  deploy_program: chunk the ELF, claim accounts, write the header
│   └── bin/hello_world.rs      deploy the program, call it, say where to read the result
├── guest/                      the on-chain program. Its own workspace.
│   ├── Cargo.toml
│   └── src/bin/hello_world.rs
└── scripts/
```

### Two programs, not one

`src/` and `guest/` are different programs, for different machines:

| | `src/` | `guest/` |
|---|---|---|
| Runs on | your machine | on-chain, inside LEZ |
| Target | native | `riscv32im-risc0-zkvm-elf` |
| Built by | `cargo build` | `cargo risczero build`, inside a fixed container |
| May use | tokio, the wallet, the network | no I/O, no network |
| Identity | just a file | **its ELF hash is the program id** |

`guest/` is the contract; `src/` is the client that deploys and calls it. They cannot be one
crate: different target, different toolchain, and the guest cannot depend on `tokio` or
`wallet`. `guest/` is a separate workspace so a host `cargo build` does not drag RISC-V code
into its graph.

The guest is built in a pinned container because a program's id *is* the hash of its ELF, so
the build has to be reproducible. Ours hashes to a different id than upstream's identical
logic, because the crate name and paths differ — which is a neat demonstration of how LEZ
addresses programs.

## What the devnet is

**Bedrock is the Logos base chain, the layer underneath LEZ.** LEZ is not standalone; it is
an *execution zone* that posts to bedrock. The sequencer executes LEZ transactions and
publishes LEZ blocks as inscriptions into a channel on bedrock; the indexer reads that
channel back. Sequencers stake into bedrock and can be slashed. Bedrock supplies consensus
and data availability, LEZ supplies execution.

| Component | Where | Port |
|---|---|---|
| bedrock | Docker, from a published release | 18080 |
| indexer | `.devnet/bin` | 8779 |
| sequencer | `.devnet/bin` | 3040 |

### Why bedrock comes from a release

Upstream resolves bedrock's binary from LEZ's own `Cargo.lock`, accepts a published release
only when tag, checksum and commit all match, and source-builds otherwise — about eight
minutes, and it needs the checkout. It source-builds more often than you would expect,
because LEZ can pin a `logos-blockchain` commit newer than the newest release. It did for
`v0.3.0-rc1`.

So this uses the latest release instead, which makes the image self-contained and quick. The
node is then *slightly behind* the revision LEZ was tested against: for `v0.3.0-rc1` that is
one commit, and it works. If a future LEZ bump breaks the channel or the sequencer starts
rejecting blocks, suspect that first, and use upstream's `just resolve-bedrock-node` to
build the exact pinned revision.

Version pins live in two places, deliberately, because they come from different repos:
`LEZ_TAG` in `scripts/common.sh`, and `BEDROCK_VERSION` in `docker-compose.yml` (override
with `BEDROCK_VERSION=0.3.0-rc.3 docker compose up --build`).

## How the program works

`guest/src/bin/hello_world.rs` appends its instruction bytes to its own shard on the single
account it is handed:

```rust
fn plan(input: &PlanInput, instruction: Instruction) -> Plan {
    let mut plan = Plan::new(input);
    let [account] = input.accounts.as_slice() else {
        panic!("Input accounts should consist of a single account");
    };
    plan.effect(account, &instruction);
    plan
}

fn apply(greeting: Vec<u8>, pre_data: &ShardData) -> Option<Vec<u8>> {
    let mut bytes = pre_data.clone().into_inner();
    bytes.extend(greeting);
    Some(bytes)
}
```

Three properties of LEZ show through.

**Programs are stateless.** Nothing persists in the bytecode; everything lives in accounts
passed explicitly into each call. One deployed program backs any number of independent
instances, which are just different sets of accounts. Deploying therefore means writing the
ELF into accounts — one per 96 KiB segment — and creating a header account that points at
that chain. The header's id addresses the program afterwards.

**The program cannot tell public from private.** Privacy is the protocol's business, decided
by whoever builds the transaction. The same bytecode runs on-chain for a public account and
locally under a RISC0 proof for a private one. Public execution takes seconds; upstream puts
the private path at 30 seconds to 4 minutes.

**`plan` declares its effects up front,** which is how LEZ schedules in parallel: calls
naming different accounts run simultaneously. Contention is per account, so a program
funnelling every call through one shared account serialises itself.

This program never checks `is_authorized`, so anyone may append to its shard on any account.
Fine for a greeting, wrong for anything holding value.

## Upstream defects this routes around

Upstream's `examples/program_deployment` tutorial cannot be followed to completion as
written. Ten defects stand between a fresh checkout and a greeting; eight stop you outright,
and one is a code bug rather than a documentation problem.

| # | Defect |
|---|---|
| 1 | `just run-bedrock` runs `(cd .. && just resolve-bedrock-node)`, and no Justfile exists above the repo root, so it fails with `no justfile found` |
| 2 | `cargo risczero build` as documented uses a guest-builder image carrying rustc 1.88.0-dev, while `enum-ordinalize` and `ruint` need 1.89/1.90 — fixed by `RISC0_DOCKER_CONTAINER_TAG=r0.1.91.1`, the tag upstream's own Justfile pins |
| 3 | The README says `cargo install --path wallet`; the crate is at `lez/wallet` |
| 4 | Indexer and sequencer both default RocksDB to `./rocksdb-<channel-id>`; started from the repo root as documented, the second dies on the first's lock |
| 5 | `just run-wallet` sets `LEE_WALLET_HOME_DIR=$(pwd)/configs/debug`, a path that does not exist |
| 6 | `wallet deploy-program <bin>` does not exist; it is `wallet program-loader deploy --elf … --header … --segments … --payer …` |
| 7 | The runners take a third argument, a funded fee payer, that the README never mentions |
| 8 | **Every upstream runner builds its message with `Message::try_new`**, documented as fee-exempt and "correct for system transactions". An ordinary program call is charged, so all of them are rejected with `InvalidParams: "Incorrect fee"`. Use `WalletCore::send_pub_tx_paid_by` |
| 9 | `wallet account get --scope all` reports `Account is Uninitialized` even when the shard holds data — and the tutorial tells you to use `all`. Reads must name the program |
| 10 | The example README documents a `ShardStateDiff` / `ProgramOutput::new(…).write()` API; the guest actually uses `run_program(plan, apply)` |

Three more things that cost time without being bugs:

- **`SUPPRESS_VERBOSE_PRINTS=1`.** Without it the wallet prints roughly 1.2 MB of raw
  transaction bytes per call.
- **Never export `RISC0_DEV_MODE` globally.** It is part of the environment risc0's build
  scripts hash, so toggling it invalidates them and forces guest rebuilds. The scripts pass
  it only to processes that execute programs.
- **Bedrock source builds can be OOM-killed** (exit 137) when cargo links many crates at
  once inside Docker's memory limit. Avoided here by not source-building at all.

## Reading state by hand

```bash
export LEE_WALLET_HOME_DIR=$PWD/.devnet/wallet
export SUPPRESS_VERBOSE_PRINTS=1
W=.devnet/bin/wallet

$W account list
$W account get --raw --scope <program-account-id> --account-id Public/<target>
echo 486f6c61206d756e646f21 | xxd -r -p     # -> Hola mundo!
```

The wallet prompts for a password on every call and derives all account keys from it. The
scripts use `program-tutorial`. It is a throwaway devnet wallet; so are the genesis keys in
`configs/`.

## Status

Verified green on macOS arm64 (M-series), Docker 28.5.2, 2026-09-26: `reset.sh` → `up.sh` →
`hello.sh` from genesis, ending in `PASS`. Standalone operation confirmed by moving the
`logos-execution-zone` checkout aside and repeating the cycle.

Timings: this project 4m39s cold, LEZ service binaries ~8m, bedrock image ~1m, devnet
healthy 10-20s after start, deploy plus call about 25s across five blocks — four ELF
segments and the header, then the call. The guest itself executes in 1.6-2.2 ms in dev mode,
worth knowing before measuring anything heavier.

Two failure modes are handled rather than waited out, because both bit during development
and neither announced itself. A `check-health` immediately after start can return
`Error: Failed to find leader` while the sequencer registers, so `up.sh` retries and logs
every attempt to `.devnet/logs/health.log`. And a submitted transaction is only in the
mempool, so its shard is unreadable until the carrying block executes — `hello.sh` polls for
the value instead of sleeping a guessed interval, and prints the failing read if it never
arrives.
