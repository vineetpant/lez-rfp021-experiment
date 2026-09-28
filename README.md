# LEZ hello world

A tiny program that runs on the [Logos Execution Zone](https://github.com/logos-blockchain/logos-execution-zone).
It compiles to RISC-V, gets deployed to a local chain, runs on-chain, and leaves
`Hola mundo!` sitting in an account. That's the whole trick — but getting there touches
every part of the stack, so it's a decent place to start something bigger.

You don't need a copy of `logos-execution-zone` to run this. Everything comes from a pinned
release.

## Run it

You'll need Docker running, [Rust](https://rustup.rs), and the RISC0 toolchain:

```bash
brew install pkg-config
curl -L https://risczero.com/install | bash
rzup install
```

Then:

```bash
./scripts/up.sh       # starts the chain, waits until it's ready
./scripts/hello.sh    # builds, deploys, calls, checks the result
```

The last thing you should see is:

```
shard hex: 486f6c61206d756e646f21
decoded:   Hola mundo!

PASS  the program ran on LEZ and the greeting round-tripped.
```

When you're done, `./scripts/down.sh` stops everything, or `./scripts/reset.sh` also wipes
the chain back to genesis. (Use `reset.sh` if you've stopped the chain and want to start
again — the sequencer won't resume onto a chain that's been wiped underneath it.)

The first run compiles a lot: roughly 5 minutes for this project, 8 for the chain's
binaries, 1 for the Docker image. After that it's all cached.

## What's in here

```
src/            the client that runs on your machine
guest/          the program that runs on-chain
docker/         the base chain, from a published release
configs/        devnet config for the sequencer, indexer and wallet
scripts/        start, stop, reset, and the demo itself
.devnet/        everything the chain writes. Gitignored.
```

Logs are in `.devnet/logs/`. Chain state and wallet keys are in `.devnet/state/` and
`.devnet/wallet/`. Nothing is written outside this folder.

## How it works

**There are two programs here, not one.** `guest/` is the contract — it compiles to RISC-V
and runs inside the chain. `src/` is the client that deploys it and calls it, and runs on
your machine. They can't be one crate: different target, different toolchain, and the guest
has no network or I/O at all.

**A program's identity is the hash of its compiled bytes.** There's no name or address you
pick. That's why the guest is built inside a fixed container — the build has to come out the
same every time. Our program hashes to a different id than the identical logic in
`logos-execution-zone`'s own example, just because the crate name differs.

**Deploying is really just writing data.** A LEZ program holds no state, so there's no
"deploy" operation as such. The 348 KB ELF gets split into four 96 KiB chunks, each written
into its own account, and a fifth "header" account points at the chain of them. That header's
id is what you call afterwards. Five transactions, which is why you see five block numbers go
by.

**Everything an account holds is per-program.** An account has a balance, a nonce, and a
shard for each program that's touched it. Our program appends its greeting to its own shard
and can't write anyone else's.

**The program never knows whether it's private.** Hand it a public account and the sequencer
runs the bytecode directly on-chain, no proof involved, in a couple of milliseconds. Hand it
a private account and the same bytecode runs on your laptop and produces a zero-knowledge
proof instead, which takes minutes. Privacy is decided by the caller, not the code.

**The chain sits on another chain.** Bedrock is the Logos base chain, running in Docker on
port 18080. LEZ blocks are written into it as inscriptions — the sequencer batches
transactions and posts them there, and the indexer reads them back out. Bedrock handles
ordering and availability; LEZ handles execution.

Here's the program itself, all of it:

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

`plan` says up front which accounts will be touched, which is how the chain runs unrelated
transactions in parallel. `apply` does the work. Note it never checks whether the caller was
authorised — fine for a greeting, wrong for anything holding value.

## Who does what

| | In rollup terms | Notes |
|---|---|---|
| **bedrock** | the L1 — settlement and data availability | a full chain with its own consensus, validators and staking |
| **LEZ** | the rollup | has blocks and state, but borrows ordering and finality from bedrock |
| **sequencer** | sequencer, executor and batch poster in one | stakes into bedrock to earn the right to post; a committee shares turns, this devnet runs one |
| **indexer** | the derivation pipeline | rebuilds LEZ state by reading bedrock back, rather than by gossip |
| **wallet** | user client and signer | not part of consensus |

Bedrock's validators and the LEZ sequencer committee are different sets of people. The
sequencer stakes *into* bedrock; it doesn't validate it.

## Zones, sequencers and validators

**A zone is a channel on bedrock plus the state behind it.** One bedrock can host several.
`logos-execution-zone` ships demos that boot two zones on a single bedrock and pass messages
between them — the receiving zone's indexer re-derives and verifies the message before acting
on it — plus a bridge that locks a balance on one zone and mints a wrapped token on the other.

**Sequencers work as a committee, one per channel.** Membership comes from bedrock: stake
enough and your key is accredited. Turns rotate round-robin. Both the stake threshold and the
turn timing are fixed at genesis in `channel_params`, and every node on the channel has to
agree on them. A sequencer can unstake and leave after a delay, and misbehaviour — posting
something that isn't a valid block, say — is slashable against that bedrock stake.

This devnet runs a single sequencer, so there is no rotation to watch. Upstream's own devnet
runs four on one channel.

**Bedrock's validators are a separate set.** They run bedrock's proof-of-stake consensus and
finalise the inscriptions LEZ posts, without looking inside them. Sequencers stake into
bedrock but take no part in validating it.

## What happens when you send a transaction

1. The **wallet** builds the message — program, accounts, nonces, fee — signs it, and sends it
   to the sequencer on `:3040`.
2. The **sequencer** checks the fee, signatures and nonces, then queues it.
3. The **sequencer executes** the program's RISC-V code, in about 2 ms, and works out the new
   state.
4. It **batches** the result into a LEZ block — up to 20 transactions, 1 MiB, or 15 seconds.
5. It **posts that block into bedrock** as an inscription on channel `0101…01`.
6. **Bedrock** orders and finalises it through its own consensus. LEZ gets no say.
7. The **indexer** polls bedrock every second, reads the channel, rebuilds the LEZ blocks and
   serves reads on `:8779`.

Execution happens at step 3, before anything is posted. So the sequencer knows the answer
early; bedrock only later agrees on where it sits in the order. A LEZ transaction is as final
as the bedrock transaction carrying it.

## Deploying a program

Two different ids are involved, and they come from different places.

**The ImageID** is the hash of the compiled ELF, worked out at build time. Identical source and
toolchain always give the same hash. Ours is `c75ffd03…`.

**The program id is an account id** — the "header" account the program gets written into. Your
wallet picks it locally, before the deploy transaction is even built. That's the id you call
afterwards.

Deploying writes the ELF into accounts: 96 KiB per segment, one account each, write-once, then
a header account pointing at the chain of them. `program_loader` then recomputes the ImageID by
walking those stored segments, so the chain never takes your word for what you deployed.

Consequences worth knowing:

- **Deploy once, call forever.** The program holds no state, so one deployment serves every
  account. `hello.sh` redeploys each run only because it's a self-contained demo.
- **Two deployments of identical code are two separate programs**, with separate shards. Same
  ImageID, different header account.
- **There's no registry.** Nothing maps a name to a program. You record the header id and
  publish it; callers hardcode or configure it.

## Client and program, side by side

| | The client (`src/`) | The program (`guest/`) |
|---|---|---|
| Chooses | which accounts to touch, what the instruction says, who pays | nothing — it reacts |
| Knows | your keys, the sequencer's address | its own id, the accounts it was handed, whether each was authorised |
| Can't | execute anything itself | reach the network, a file, or a clock |

The runtime calls the program **twice**: `plan` first, to declare which accounts will be
touched, then `apply` to produce the new bytes. The declaration is what lets the chain run
unrelated transactions in parallel.

Three things the program does *not* get:

- **No `msg.sender`.** `caller_account_id` is the calling *program* in a chained call, and
  `None` for a direct one.
- **No owner and no deployer.** Nothing records who paid to deploy it. An admin pattern means
  storing an account id in your own shard and checking it yourself.
- **No say in authorisation.** The runtime verifies signatures and hands the program a boolean
  per account. Whether that boolean matters is the program's choice — ours ignores it, which is
  why anyone can append to its shard on any account.

## Looking around yourself

```bash
export LEE_WALLET_HOME_DIR=$PWD/.devnet/wallet
export SUPPRESS_VERBOSE_PRINTS=1
W=.devnet/bin/wallet

$W account list
$W account get --raw --scope <program-id> --account-id Public/<target>
```

The password is `program-tutorial`. It seeds every key in the wallet, so the same password
always gives you the same accounts. It's a throwaway devnet — so are the genesis keys in
`configs/`.

Two things that will trip you up:

- **Reads have to name the program.** `--scope all` will tell you the account is
  uninitialised even when the data is right there. Pass the program's id instead.
- **A submitted transaction isn't a finished one.** It sits in the mempool until a block
  carries it, so give it a moment before reading, or poll like `hello.sh` does.

## Where this goes next

This repo exists to become an RFP-021 module — trustless Ethereum state attestation. Sketching
how that lands in the model above, since it's what the shape is for:

**Deployment.** The attestation program is deployed once through `program_loader`, like this
one, and its header id is published. Consumers hardcode it. RFP-021 is explicit that the
verification logic must not be reachable through a program id kept in mutable state — whoever
controls that pointer could otherwise swap in a module that attests to anything. Upgrading
means deploying a new program and having consumers migrate on purpose.

**An instance is a set of accounts.** Programs are stateless, so the tracked Ethereum
checkpoint, the chain config and the current sync-committee state all live in accounts, likely
program-derived ones. One deployed program can back a mainnet instance and a testnet instance
at once; only the accounts differ.

**Immutability is structural.** The checkpoint is the single trust input, set when the instance
is initialised. Nothing may rewrite it afterwards — and since there's no owner and no admin
unless a program invents one, the way to guarantee that is simply never to write an instruction
that mutates it. Absent code can't be called. Note that `program-loader deploy --immutable` is
a self-declaration and explicitly *not* protocol-enforced, so it isn't the mechanism.

**Two kinds of call.** Anyone may advance the consensus state — one public transaction per
sync-committee period, permissionless, paid by whoever submits it. Separately, a consuming
program asks for an attestation: it supplies a header, the branches and Merkle proofs, and a
predicate, and gets back a proven statement plus an identifier it can key a nullifier on. The
expensive consensus check happens once per period and is shared; the cheap inclusion check
happens inside the consumer's own transaction, so verification and the action it authorises
can't come apart.

For comparison, some programs *are* built into the node with fixed ids — the fee, clock, bridge
and sequencer-stake programs work that way. RFP-021 doesn't ask for that; it describes a shared
module deployed once, which consumers converge on.

## Status

Works end to end on macOS arm64 with Docker 28.5.2, last checked 2026-09-26. Pinned to LEZ
`v0.3.0-rc1`; the base chain's version is set separately in `docker-compose.yml`.

`CLAUDE.md` has the reasoning behind the setup and what's planned next.
