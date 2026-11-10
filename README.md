# Trustless Ethereum state attestation on LEZ

This repo tests one approach to [RFP-021](https://github.com/logos-co/rfp) (Trustless Ethereum
State Attestation for LEZ). It runs on a real [Logos Execution Zone](https://github.com/logos-blockchain/logos-execution-zone)
v0.4 node (Bedrock, sequencer and indexer) on a local devnet, against live Ethereum mainnet
data, and measures what each step costs.

**The approach.** The light client is run by an operator off-chain. LEZ holds the result and
only accepts it with a proof.

1. **Operator, off-chain.** The operator follows Ethereum: it fetches sync-committee updates
   and finality updates from a Beacon API node, and storage proofs (`eth_getProof`) from an
   execution node. It needs no trust in either source.
2. **Update, private.** For each update, the operator runs the `eth_lc` program's
   verification in its own private actor: the sync-committee BLS signature, the participation
   threshold, and the finality and handoff branches. It proves that run locally and submits
   only the proof. The sequencer verifies the proof without re-running the BLS check. The
   program then advances the public **instance** account, which accepts the new store only
   from the program itself and only on top of the store it already holds.
3. **Attestation.** Each verified header (execution state root, receipts root, block number)
   is written once to its own **record** account, at an address derived from the instance and
   the slot, and never changed. A consumer proves a fact about Ethereum, such as a storage
   slot's value, against one record:
   - **public:** the sequencer runs the Merkle-Patricia check and emits the statement;
   - **private:** the consumer runs the check in its own proof. Only a "this instance holds
     this header" check is public. The query, the proof and the result stay private.

What we measure on the node: the fee of every transaction, and the proving time and peak
memory of every private step. Fees are measured from balances, because v0.4 has no receipts.

**`hello_world` tests nothing the RFP asks for.** It's a minimal program that leaves
`Hola mundo!` in an account, called once publicly and once privately. It's a smoke test, run
first to check that the node, the wallet, deployment and both transaction paths work at all.
It also gives one number used below: a private transaction costs 5,070,616 tokens even when
the program does nothing. The light client, the updates and the attestations are all
`eth_lc`, run by `scripts/lc.sh`.

## Who runs what

`eth_lc` is **one program**: one RISC-V binary (`lc/guest`), deployed once on LEZ and
immutable. LEZ v0.4 runs a program's code in one of two places, and `eth_lc` uses both:

- **Publicly**, on a public account: the **sequencer** runs the code itself, and everyone sees
  the inputs and the result.
- **Privately**, on a private account: the **sender's wallet** runs the code on the sender's
  own machine and proves it ran (RISC Zero). Only the proof, and any calls the code makes to
  public accounts, reach the sequencer. The sequencer checks the proof instead of re-running
  the code, then runs those public calls itself.

Each `eth_lc` message is written for one of the two:

| Message | Runs where | Does what |
|---|---|---|
| `Init` | sequencer, on the **instance** account | checks a trusted checkpoint and starts the light-client store |
| `Update` | operator's machine, proven | verifies a sync-committee update (BLS signature, participation, finality and handoff branches), then calls `Advance` |
| `Advance` | sequencer, on the instance | accepts the new store only from `eth_lc` itself, on its config and on top of the store it holds; then calls `WriteRecord` |
| `WriteRecord` | sequencer, on a new record account | writes the header once, only from `eth_lc` itself, only at the address its instance and slot derive |
| `Attest` | sequencer, on the record account | checks a Merkle-Patricia proof against that record |
| `AttestPrivate` | consumer's machine, proven | checks the same proof privately, then calls `CheckRecord` |
| `CheckRecord` | sequencer, on the record account | refuses unless the account holds that instance's record of that header |

**The instance** is a public account whose state is the light client: the config fixed at
`Init`, the current store and its latest record. Only `eth_lc` code can change it. Each
**record account** holds one verified header, written once.

**The operator** isn't simulated by a program on LEZ. It's ordinary off-chain software: our
client `src/bin/eth_lc.rs` and a LEE wallet, run on the laptop. For each update it:

1. fetches the update from a public Beacon API node, and checks it natively (`eth_lc check`);
2. builds an `Update` message;
3. has the wallet run `eth_lc`'s `Update` code on the operator's private account, prove it,
   and submit the proof (about 36 minutes and 9 GB on the laptop);
4. reads the instance back from the sequencer to confirm it advanced.

So the light-client logic is split. The operator does the fetching and the proving off-chain.
LEZ verifies the proof, enforces `Advance`'s rules, and holds the result. The operator is
trusted for liveness only: a wrong update can't produce a valid proof, and an operator that
stops just means the instance stops advancing.

**The consumer** is the same client in another role. It fetches an `eth_getProof` from a
public execution node and sends either a public `Attest` (a plain transaction) or an
`AttestPrivate` (proven on its machine, about 15 minutes and 8 GB).

**What this test simplifies:**

- **Roles:** one laptop and one wallet play operator, consumer and fee payer.
- **Consumers:** no other LEZ program consumes an attestation. The consumer is a person sending
  a transaction, not a contract acting on the result.
- **Network:** the LEZ network is a local devnet with one sequencer and one Bedrock node, not
  a shared testnet.

The proposal, the cost analysis behind it, and every measurement are in
[`rfp-021/`](rfp-021/).

You don't need a copy of `logos-execution-zone` to run any of this. Everything is pinned:
LEZ to `v0.4.0-rc1`, and Bedrock to the exact commit that tag's lockfile resolves.

## Run it

You'll need [Rust](https://rustup.rs), the RISC Zero toolchain, and Docker (only to build the
programs in a pinned container):

```bash
brew install pkg-config
curl -L https://risczero.com/install | bash
rzup install
```

Then:

```bash
./scripts/up.sh       # builds and starts bedrock, the sequencer and the indexer
./scripts/hello.sh    # hello world, publicly then privately
./scripts/lc.sh       # the light client, end to end on live mainnet data
```

`hello.sh` ends with:

```
  actor state: 486f6c61206d756e646f21 = "Hola mundo!"

PASS  the program ran on LEZ publicly and privately, and the greeting round-tripped both ways.
```

and `lc.sh` with:

```
PASS  eth_lc tracked live mainnet on LEZ and attested a storage slot publicly and privately.
```

**Dev mode or real proofs.** By default every proof is faked, which takes seconds and checks
the plumbing. The node accepts fake proofs in that mode, so it verifies nothing. For real
proofs, verified by the node, set `DEV_MODE=0` for all three scripts:

```bash
./scripts/reset.sh
DEV_MODE=0 ./scripts/up.sh && DEV_MODE=0 ./scripts/hello.sh && DEV_MODE=0 ./scripts/lc.sh
```

That takes about two hours on a laptop CPU, mostly the two light-client updates at about half
an hour each. Run it under `caffeinate -i` on a Mac, or a sleeping laptop pauses the proof.

Every fee the sequencer charges is printed as a `FEE` line and kept in
`.devnet/logs/fees.log`, with proving times (`TIME`) and peak memory (`MEM`).

`./scripts/down.sh` stops everything; `./scripts/reset.sh` also wipes the chain back to
genesis. The first run compiles a lot: about 13 minutes for the LEZ services, 9 for Bedrock,
10 to build `eth_lc` in its container. After that it's all cached.

## The RFP-021 approach

**The problem.** A LEZ program that acts on Ethereum (a bridge, say) needs proof that a fact
holds on finalized Ethereum, checked by LEZ itself, with no committee or oracle to trust.

**The answer: an operator runs the light client, and LEZ checks a proof of it.** Ethereum's
sync committee, 512 validators chosen per 27-hour period, signs every finalized block header. A
light client checks that signature (BLS12-381, at least two thirds of the committee) and the
Merkle branches around it, and from then on trusts the header. The operator does that check
off-chain, as the `eth_lc` program's private actor, and submits a proof of it. LEZ never runs
the check itself. It verifies the proof and records the header in the program's public state.

**One program, many instances.** An instance is the program's actor state on one public
account. It holds the chain config, fixed at `Init`, the light-client store (finalized header,
current and next committee roots), and its latest record. Every verified header also gets its
own write-once record account, so no action reads more than one record. It is
initialised once from a checkpoint the deployer trusts, and signed by that account so nobody
else can squat it.

**Updates are proven privately.** The BLS check costs about 11.4–12.5M cycles, more than a public
transaction (10M) or a whole block may use. So it runs on a **private actor** of the program:
on the submitter's machine, inside a zero-knowledge proof. The sequencer only verifies the
proof and applies the result, which costs a flat fee. Nobody has to trust the submitter: the
proof shows the update verified, and the instance accepts it only from its own program and
only on top of the store it holds.

**Attestations come in two kinds.**

- **Public:** the consumer's transaction sends `Attest` to the instance. The sequencer checks
  the Merkle-Patricia proof (a storage slot, an account, a receipt log) against a recorded
  header and emits the statement.
- **Private:** the proof check runs on the caller's private actor, so the query, the proof and
  the statement stay inside the caller's proof. Only one thing is published: a check that the
  instance holds the header record used.

**Immutable by construction.** The program is deployed immutable, which the v0.4 loader
enforces. No instruction rewrites an instance's config or checkpoint. Upgrading means
deploying a new program and having consumers move to it deliberately.

The full design, with costs, attack analysis and milestones, is in
[`rfp-021/proposal.md`](rfp-021/proposal.md).

## The flows, step by step

### A light-client update

```
operator's machine                                 LEZ sequencer
──────────────────                                 ─────────────
fetch the update from a beacon node
  (Beacon API: light_client/updates, finality_update)
verify it natively first (eth_lc check)
prove, on the operator's private actor:
  Update { store, update, committee, key witness }
    the verifier: participation, BLS signature,
    finality branch, committee handoff
    → sends Advance { cfg hash, prev, next }
                         ──── proof + Advance ────▶  verify the proof
                                                    run Advance on the public instance:
                                                      only from eth_lc itself
                                                      only on the config bound at Init
                                                      only on top of the store it holds
                                                    store := next; record the new header
```

The update transaction also carries a validity window. Past `max_checkpoint_age` (two weeks)
from the store's finalized slot, the sequencer's clock refuses it.

### A public attestation

```
consumer                                 eth_lc instance (public)
────────                                 ────────────────────────
eth_getProof at a recorded block
Attest { query, slot }  ───────────────▶ find the header record
                                         check the MPT proof against its state root
                                         check every predicate
                                         emit Attested(statement), or refuse
```

### A private attestation

```
caller's machine                                   eth_lc instance (public)
────────────────                                   ────────────────────────
prove, on the caller's private actor:
  AttestPrivate { record, query }
    check the MPT proof and predicates
    against the record
    → sends CheckRecord { record }
                    ──── proof + CheckRecord ────▶ refuse unless the record is held
```

What becomes public: that someone attested against that record, which is a public header
anyway. What stays private: which account or event, the values, and the predicates.

## How we know it worked

`hello_world` plays no part in any of this. The light client and the attestations are tested
by `scripts/lc.sh`, with `eth_lc` on the devnet node and live mainnet data.

**What is attested.** One real fact about Ethereum mainnet: a WETH contract storage slot (one
holder's balance entry) is nonzero at the block the instance last recorded. The consumer
fetches `eth_getProof` for that slot from a public execution node it doesn't trust. `eth_lc`
then checks the whole chain of evidence:

1. The header record comes from the instance. The instance got it only from a light-client
   update that verified, so it traces back to the sync committee's signature.
2. The account proof (Merkle-Patricia, keccak) has to lead from the record's **state root**
   to the WETH account, which gives its storage root.
3. The storage proof has to lead from that storage root to the slot, which gives its value.
4. The predicate (value > 0) has to hold.

If any step fails, the program panics.

**What counts as success.** Being included in a block doesn't. In v0.4 a public action that
fails is still included and charged, with its effects undone and its index added to the
block's `reverted` list. v0.4 has no receipts and exposes no events, so the client fetches the
block (`getBlock`) and checks that list. An attestation is **accepted** only when it is
included and not reverted. The updates are checked more directly: the client reads the
instance back after each one, and its finalized slot and records have to have moved.

**What must fail.** An attestation that can't fail proves nothing. So `lc.sh` follows the
honest attestations with broken ones, each breaking exactly one thing, and fails unless each
is refused:

| Case | Broken | Expected outcome |
|---|---|---|
| `tampered-proof` | one byte of the storage proof's leaf | public: included, charged, **reverted**; private: **unprovable**, because the check fails before a proof exists |
| `false-claim` | the claim: "the slot is zero" | public: **reverted**; private: **unprovable** |
| `unheld-record` | private only: a header the instance never verified, with the real state root, so the caller's own checks pass | the instance's `CheckRecord` refuses it: **unprovable** (the wallet runs that call before proving), or **reverted** or **rejected** by the sequencer |

The last case tests the private path's trust boundary. Proving a statement privately is easy
against any header the caller makes up. The only thing that ties it to verified Ethereum is
the instance confirming it holds that header.

**Dev mode against real proofs.** `DEV_MODE=1` fakes every proof, and the node accepts fakes,
so it shows the logic and the fees but not that the proofs verify. All the results below are
with `DEV_MODE=0`: every private step was actually proven on the laptop, and the sequencer
verified each proof before including it.

**What happened (real proofs, 9 Nov 2026):**

| Attestation | Public | Private |
|---|---|---|
| Honest | ✅ **accepted**, not reverted (block 366) | ✅ **accepted**, not reverted (block 425) |
| `tampered-proof` | ✅ **reverted** (block 546) | ✅ **unprovable**: `InvalidInclusionProof` |
| `false-claim` | ✅ **reverted** (block 547) | ✅ **unprovable**: `ConditionUnsatisfied` |
| `unheld-record` | — | ✅ **unprovable**: `record not held by this instance` |
| `history-forged` (a forged header for an older block) | ✅ **reverted** (block 548) | ✅ **unprovable**: `InvalidExecutionProof` |

Every honest attestation was accepted and every broken one refused. The updates were
verified by reading the instance back after each one. Two things came out of it:

- **A refused public attestation is expensive.** It's charged its whole gas limit: 16,060,112,
  against 2,469,032 for an accepted one, 6.5× as much. So the client now dry-runs the check
  first and stops on a refusal, with the same typed error the program gives. A tight gas limit
  is next ([#25](https://github.com/vineetpant/lez-hello-world/issues/25)).
- **A refused private attestation costs nothing.** The wallet runs the program before proving,
  including the public `CheckRecord` call, so a broken attestation fails in under 0.1 s and
  nothing is sent. What isn't tested is a modified wallet that skips that run and submits
  anyway, so that the sequencer itself has to refuse it
  ([#26](https://github.com/vineetpant/lez-hello-world/issues/26)).

## Measured

All numbers come from runs we watched. The node runs are on the local v0.4 devnet (one
sequencer, one Bedrock validator, base fees at their floor of 8). The offline runs are on the
same LEZ code, without a node. Details and raw logs are in
[`rfp-021/results.md`](rfp-021/results.md).

**On the LEZ v0.4 devnet node, real proofs, M3 Pro 18 GB CPU** (9 Nov 2026; base fees at 8,
fees in tokens):

| Step | Where it runs | Time | Peak memory | Fee charged |
|---|---|---:|---:|---:|
| Deploy `eth_lc` | sequencer | — | — | 5,429,088 |
| `Init` from a checkpoint, with its record | sequencer | — | — | 11,194,336 (1.37M exec gas + 26,579 B) |
| Update, committee handoff | operator proves, sequencer verifies | 36.7 min | 8.4 GB | 5,333,856 |
| Update, finality, writing a new record account | operator proves, sequencer verifies | 35.6 min | 8.7 GB | 5,526,600 |
| Public attestation, storage slot, accepted | sequencer | 12 s to inclusion | — | 2,469,032 (301k exec gas + 7,514 B) |
| Public attestation, 1,000 blocks back through EIP-2935 | sequencer | one block | — | 4,504,320 (549k exec gas + 13,863 B) |
| Public attestation, 3 blocks back through a parent-hash chain | sequencer | one block | — | 3,297,992 (402k exec gas + 10,131 B) |
| Private attestation, storage slot, accepted, at the record or an older block | consumer proves, sequencer verifies | 14.2–15.2 min | 7.3–7.8 GB | 5,232,864 |
| Public attestation, refused (reverted) | sequencer | one block | — | 16,060,112 (the whole 2M gas limit) |
| Private attestation, refused | wallet, before proving | < 0.1 s | — | 0 (nothing sent) |
| Baseline: a private call that does nothing (hello world) | — | — | — | 5,070,616 (dev mode) |

Every row except the baseline is from real proofs. Chrome and Brave were open during the run;
a first run of the same program gave update times within 2 minutes of these. Outcomes are in
[How we know it worked](#how-we-know-it-worked). Per-step reserves, refunds, gas and cycles are
in [results.md](rfp-021/results.md#node-run-real-proofs-on-a-lez-v04-devnet). The raw logs are
in [rfp-021/results/node-2026-11-09/](rfp-021/results/node-2026-11-09/).

A private update costs about the same whatever it verifies. LEZ charges a flat 409,764 exec
gas + 224,063 storage gas to verify any proof, and only the update's public part (33–57k
cycles) adds to that. The cost of the BLS check itself shows up as proving time on the
operator's machine, not as fees.

**Offline, without a node** (RISC Zero's executor at LEZ's metering, `tests/cost.rs`; see
[results.md](rfp-021/results.md#cycles-per-action)):

| | Result |
|---|---|
| Update, committee handoff | 12.53M cycles |
| Update, finality only | 11.39M cycles |
| The same update run publicly | over one transaction's 10M cap, so two transactions in two blocks, about 25× the proven path's exec gas |
| `Init` | 1.36M cycles |
| Attestation, storage slot | 0.30M cycles |

On our earlier prototype verifier, proving one update took 30.6 min at the default segment size
(swapping), 34.6 min in 5.3 GB at 2^19 and 48 min in 3.9 GB at 2^18; the BLS pairing was 61% of
its cycles and hash to G2 9%; a receipt-log attestation was 52–69k cycles. These are re-measured
on the current verifier in M0.

## What's in here

```
src/            the clients: hello_world, eth_lc, and fee measurement
guest/          the hello_world program
lc/guest/       the eth_lc program
lc/types/       eth_lc's messages, state and checks, shared by program, client and tests:
                the verifier (core), inclusion proofs, statements, history, Helios oracle
tests/          native tests on recorded mainnet: against Helios, adversarial, history,
                statements, cycle costs
configs/        devnet config for bedrock, the sequencer, indexer and wallet
scripts/        start, stop, reset, the two demos, live chain fetching
rfp-021/        the proposal, the analysis, measurements and results
.devnet/        everything the chain writes, and the binaries. Gitignored.
```

Logs are in `.devnet/logs/`. Chain state and wallet keys are in `.devnet/state/` and
`.devnet/wallet/`. Nothing is written outside this folder.

## Credit

`eth_lc`'s verifier is ours: the consensus specs' light-client protocol
(`validate_light_client_update`, `apply_light_client_update`), about 300 lines in
`lc/types/src/core.rs`. It is built on, and checked against, these:

- **`blst`** ([supranational/blst](https://github.com/supranational/blst), Apache-2.0) checks
  the sync-committee BLS12-381 signature. It was audited by NCC Group for the Ethereum
  Foundation, and Galois is formally verifying it. The program uses RISC Zero's accelerated
  fork.
- **Helios** ([a16z/helios](https://github.com/a16z/helios), MIT) `consensus-core` is the
  verifier's reference. In the tests, every recorded mainnet update and every attack case goes
  through both, and both must agree (`tests/oracle.rs`, `tests/adversarial.rs`). Its consensus
  logic was audited by zkSecurity. The client also uses its types to read Beacon API JSON.
  Helios isn't in the program.
- **RISC Zero** accelerated forks of `blst`, `sha2` and `tiny-keccak`, and **alloy**
  (`alloy-trie`, `alloy-rlp`, `alloy-consensus`) for Merkle-Patricia proofs and receipts.

What this repo builds is the program on the LEZ v0.4 actor model, plus the tooling around it:

- the update verified privately, in one proof, rather than split across public transactions;
- the `from_own_program` boundary between the program's actors;
- header records in write-once accounts derived from the instance and slot;
- private attestation through `CheckRecord`;
- attestations to older blocks through EIP-2935 and parent-hash chains;
- the clients, the devnet, and the measurements.

## How LEZ works, through these two programs

**There are two kinds of program here.** `guest/` and `lc/guest/` are contracts. They compile to
RISC-V and run inside the chain. `src/` holds the clients that deploy and call them, and runs on
your machine. They can't be one crate: different target, different toolchain, and a contract
has no network or I/O at all.

**A program's identity is the hash of its compiled bytes.** There's no name or address you
pick. That's why the programs are built inside a fixed container: the build has to come out
the same every time.

**Deploying is just writing data.** A LEZ program holds no state, so there's no "deploy"
operation as such. The program's ELF is split into 96 KiB chunks, each written into its own
account, and a "header" account points at the chain of them: five transactions for hello
world, more for `eth_lc`. That header's id is what you call afterwards.

**State is actor state.** An account has a balance, a nonce, and one slot of state for each
program that has written to it: the *actor state* of that (account, program) pair. Only that
program can write it. `hello_world` appends to its own; an `eth_lc` instance is `eth_lc`'s actor
state on one account.

**Programs talk by messages.** A program receives a message, sees its own actor state and who
sent it, and responds with new state and messages to other actors. That's how a private
actor's `Update` hands its result to the public instance's `Advance`. A receiver can tell when
the sender is another actor of its own program (`from_own_program`), which is the trust
boundary `eth_lc` is built on.

**The program never knows whether it's private.** Send a message to a public account and the
sequencer runs the program directly. Send it to a private account and the same bytecode runs
on your laptop under a zero-knowledge proof, and only the proof reaches the chain. Privacy is
decided by the caller, not the code. A private transaction pays its fee inside the proof, from
a private account, which is why `hello.sh` first shields some tokens into one.

**The chain sits on another chain.** Bedrock is the Logos base chain, on port 18080. LEZ blocks
are posted into it as inscriptions: the sequencer batches transactions and posts them there,
and the indexer reads them back out. Bedrock handles ordering and availability; LEZ handles
execution.

| | In rollup terms | Notes |
|---|---|---|
| **bedrock** | the L1: settlement and data availability | a full chain with its own consensus, validators and staking |
| **LEZ** | the rollup | has blocks and state, but borrows ordering and finality from bedrock |
| **sequencer** | sequencer, executor and batch poster in one | stakes into bedrock to earn the right to post; a committee shares turns, this devnet runs one |
| **indexer** | the derivation pipeline | rebuilds LEZ state by reading bedrock back, rather than by gossip |
| **wallet** | user client, signer and prover | not part of consensus |

### What happens when you send a transaction

1. The **wallet** builds the message (program, actors, nonces, fee), signs it, and for a
   private one proves it locally. Then it sends it to the sequencer on `:3040`.
2. The **sequencer** checks the fee, signatures, nonces and any proof, then queues it.
3. The **sequencer executes** the public part and works out the new state.
4. It **batches** the result into a LEZ block: up to 20 transactions, 1 MiB, or 15 seconds.
5. It **posts that block into bedrock** as an inscription on channel `0101…01`.
6. **Bedrock** orders and finalises it through its own consensus.
7. The **indexer** reads the channel back, rebuilds the LEZ blocks and serves reads.

**Fees** are charged in two dimensions. Exec gas is the cycles the public part ran; storage gas
is the transaction's bytes. A private transaction instead pays a flat 409,764 exec gas for
verification, 224,063 storage gas, and any cycles its public part runs.

## Looking around yourself

```bash
export LEE_WALLET_HOME_DIR=$PWD/.devnet/wallet LEE_WALLET_PASSWORD=program-tutorial
export SUPPRESS_VERBOSE_PRINTS=1
W=.devnet/bin/wallet

$W account list
$W account get --raw --scope <program-id> --account-id Public/<account>
./target/release/eth_lc show <eth_lc-program-id> <instance>
```

The password seeds every key in the wallet. It's a throwaway devnet, and so are the genesis
keys in `configs/`.

Things that will trip you up:

- **Reads have to name the program.** `--scope all` will tell you the account is
  uninitialised even when the data is right there.
- **A submitted transaction isn't a finished one.** It sits in the mempool until a block
  carries it, so poll before reading, like the scripts do.
- **A private account's state moves on with every transaction.** If the wallet misses one,
  its next transaction is refused as `Commitment … already seen`. `wallet account
  sync-private` catches it up.

## Status

Runs on macOS arm64 (M3 Pro, 18 GB), on a local LEZ `v0.4.0-rc1` devnet with Bedrock at
`d480efc`, last run 2026-11-09. The light-client updates and the public and private
attestations are verified on the node with real proofs, and every broken attestation was
refused (see [How we know it worked](#how-we-know-it-worked)). LEZ testnet 0.4, which RFP-021 targets, wasn't live yet;
deploying there is the next step once it is.

**Where `eth_lc` differs from the proposal** ([`rfp-021/proposal.md`](rfp-021/proposal.md)):

- **Instance IDs:** the proposal derives them from `H(checkpoint ‖ config)`. `eth_lc` uses any
  account, and requires `Init` to be signed by it
  ([#24](https://github.com/vineetpant/lez-hello-world/issues/24)).
- **Private attestation:** same design as the proposal: one `CheckRecord` call crosses the
  boundary, from a blinded alias the wallet picks fresh for each message, and the fee refund
  returns to the payer's blinded alias. A publication check after each private step confirms
  that no private account, payer or query detail is in the transaction
  ([results](rfp-021/results.md#node-run-real-proofs-on-a-lez-v04-devnet)).
- **Operator software:** the proposal names the Logos Ethereum core module (Nimbus
  `libverifproxy`). This test uses its own client, reading public RPC nodes with Helios's types
  ([#29](https://github.com/vineetpant/lez-hello-world/issues/29)).
- **Not exercised here:**
  - the public split update path;
  - a racing or deferral policy between operators;
  - a consumer program that builds the predicate.

**Attestation against the RFP's requirements**, as of the 9 Nov node run. Numbers are the RFP's
Functionality (F), Reliability (R), Performance (P) and Success (S) items.

| Requirement | Public | Private | Issue |
|---|---|---|---|
| F.2 Header finalized via sync-committee updates | ✅ record taken from the instance | ✅ record checked by `CheckRecord` | |
| F.3 Inclusion: account and storage slot | ✅ runs on the node | ✅ runs on the node | |
| F.3 Inclusion: event log | ⚠️ native test only | ⚠️ native test only | |
| F.3 Caller roots never trusted, bad claims refused | ✅ roots from the instance; tampered proof and false claim reverted | ✅ tampered proof, false claim and unheld header all refused; the sequencer-side `CheckRecord` refusal isn't tested | [#26](https://github.com/vineetpant/lez-hello-world/issues/26) |
| F.4 Predicate, set by the consuming program | ⚠️ the conditions `= ≠ < ≤ > ≥` exist; the consumer interface is open | ⚠️ same | [#13](https://github.com/vineetpant/lez-hello-world/issues/13) |
| F.5 Chain ID bound, cross-chain rejection | ⚠️ in the record; refused natively, not tested across chains on the node | ⚠️ same | [#27](https://github.com/vineetpant/lez-hello-world/issues/27) |
| F.6 Nullifier-safe statement ID | ⚠️ not tested | ⚠️ not tested | [#13](https://github.com/vineetpant/lez-hello-world/issues/13) |
| F.7 Attestation verified inside the consumer's own transaction | ❌ no consumer program; the result is only an event | ❌ the result goes nowhere | [#13](https://github.com/vineetpant/lez-hello-world/issues/13) |
| F.8 No correlation signal, checked over the full diff | n/a: public by design | ✅ in the transaction: the sender is a one-time alias, and no private account, payer or query detail appears; the header used is revealed by design. ⚠️ full public-state diff not done | [#7](https://github.com/vineetpant/lez-hello-world/issues/7) |
| R.2 A rejected attestation consumes nothing | ⚠️ consumer state is untouched, but a refused attestation is charged its whole gas limit (16.06M) | ✅ a broken one can't be proven, so nothing is sent | [#25](https://github.com/vineetpant/lez-hello-world/issues/25) |
| R.3 A valid statement stays valid forever | ✅ records are written once and never removed | ✅ same | |
| R.6 Reuse audited implementations | ✅ verifier on audited `blst`, held to Helios on every case | ✅ same | |
| Soft #3 Attest to older state | ✅ a block 1,000 back through EIP-2935, 3 back through a parent-hash chain; a forged header refused | ✅ same, at no extra charge | [#28](https://github.com/vineetpant/lez-hello-world/issues/28) |
| P.1 Fits one transaction's budget | ✅ 301k exec gas at the record, 549k for a block 1,000 back: 3–5.5% of the cap | ✅ small public part | |
| S.1 Deployed on testnet 0.4 | ❌ local devnet; testnet not live | ❌ | [#15](https://github.com/vineetpant/lez-hello-world/issues/15) |

In short, the attestation core (finality plus storage-slot inclusion) works on both paths. It's
accepted on the node when honest, and refused when broken. What's missing
for compliance is mostly on the consumer side: the predicate interface, attesting inside the
consumer's transaction, event logs on the node, statement IDs and the chain-ID test.

Not yet done: receipt-log attestations on the node, a consumer program composing with
`eth_lc`, catch-up across several periods, `historical_summaries`, and GPU proving. They're
tracked as issues. `CLAUDE.md` has the reasoning behind the setup.

The v0.3 hello world is tagged `v0.3-hello`.

## Licence

MIT OR Apache-2.0.
