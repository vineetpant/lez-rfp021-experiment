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

## Status

Works end to end on macOS arm64 with Docker 28.5.2, last checked 2026-09-26. Pinned to LEZ
`v0.3.0-rc1`; the base chain's version is set separately in `docker-compose.yml`.

`CLAUDE.md` has the reasoning behind the setup and what's planned next.
