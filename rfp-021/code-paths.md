# RFP-021 on LEZ: code paths and what they cost

Working notes for our proposal. Read against `logos-execution-zone` `dev` @ `952bd6dd`
(`v0.4.0-rc1` plus CI-only commits), the runtime RFP-021 targets (testnet 0.4). File references
are to that checkout.

The first version of these notes was written against `v0.3.0-rc1` (plan/apply). v0.4 replaced
that model, so this version is rewritten on actors. The v0.3 text is in the history of
the old standalone `../rfp-021` repo (`a024c15`). Section 7 lists what carried over and what didn't.

Every number below is marked as one of:

- **measured (prototype)**: our earlier prototype verifier on LEZ `v0.4.0-rc1` (M3 Max, 16
  cores, 64 GB, CPU only, or the reference laptop where stated); re-measured on the delivered
  verifier in M0
- **measured**: our runs of the current verifier, in [results.md](results.md)
- **measured (LEZ v0.3)**: `docs/benchmarks/cycle_bench.md` (M2 Pro, CPU only, risc0 3.0.5),
  taken on v0.3. Proving-pipeline numbers should be similar on v0.4, but they were not re-run
- **derived / projected**: our arithmetic from measured numbers, not run by anyone
- **to measure**: in [bench-plan.md](bench-plan.md)

## 1. Execution model: actors

An **actor** is one program's state inside one account:
`Actor { account_id, program_account_id }` (`lee/state_machine/core/src/account.rs`). A program
is a single message handler (`lee_core::actor_entrypoint!`, `program/mod.rs`):

```
handle(ReceiveInput) -> Response

ReceiveInput { receiver, from: Option<Actor>, is_authorized, pre_state, message }
Response     { post_state, sync_messages, async_messages, events, validity }
```

- The handler sees **its own `pre_state`** and nothing else. To change or read another actor,
  it sends that actor a message.
- `SyncMessage` is handled in the same transaction. `AsyncMessage` is handled in a later one, by
  whatever program is deployed at the destination at that time.
- `from` is the sending actor, or `None` when the transaction submitted the message.
  `from_own_program()` is true when the sender's `program_account_id` equals the receiver's.
- Reading state is a per-program convention: a handler answers a `ReadState` message with a
  `StateReply { state: pre_state }`. The native token does this. It is not a protocol feature.
- Validity windows (`timestamp_window`, `block_window`) are set on the `Response`.
- PDAs still exist (`for_public_pda`, `for_private_pda`), and so do blinded accounts.
- Programs are **upgradable** unless deployed with `ProgramHeader.immutable = true`
  (`program/mod.rs:523`). In v0.4 the loader enforces it: `UpdateHeader` asserts
  `!old_header.immutable` (`lez/programs/program_loader/core/src/lib.rs:93`). In v0.3 the flag
  was documented as self-declared and not enforced.

### v0.3 → v0.4 mapping

| v0.3 (plan/apply) | v0.4 (actors) |
|---|---|
| `plan` + `apply` entry points | One handler returning a `Response` |
| A program's shard of an account | That actor's `ActorState` |
| Plan can't read state; the caller passes it in and apply checks a digest | The handler gets its own `pre_state` and compares directly (`assert_eq!(pre_state, prev)`) |
| One plan emits effects on many accounts | Each actor writes only itself; other accounts are changed by messaging their actors |
| `ChainedCall` with a return | `send_sync`, plus a reply that arrives as a new message; multi-step logic keeps pending state |
| Public applies deferred from a private transaction | Messages crossing to public actors are published, and those actors run on-chain, metered |

## 2. Public transaction path

```
wallet ─► sequencer: handle message (guest, metered) → sync messages → … ─► state diff
          ─► block ─► Bedrock ─► indexer
```

Fees (`lez/programs/fee/core/src/market.rs`, `assess.rs`):

- Billed `min(cycles, gas_limit)` plus storage gas per byte.
- `MAX_GAS_EXEC` = 10M caps both a transaction's gas limit (`fee/core/src/validity.rs`) and a
  block's total (`fee/src/main.rs`). Target is 5M. `MAX_GAS_STOR` = 1M.
- A refused transaction is still included and charged its full gas limit.

## 3. Private (PPE) transaction path

Prover side (`lee/state_machine/src/privacy_preserving_transaction/circuit/mod.rs`):

1. `execute_and_prove` walks the message tree with the `Prover` backend.
2. Each private program execution is proven as its own risc0 receipt (`prove_session`) and
   added as an assumption.
3. The privacy circuit `env::verify`s each one against the claimed image ID.
4. The outer proof is `ProverOpts::succinct()`, a fixed 223,551 bytes.
5. There is no way to attach an external receipt. A program **cannot** verify an outside ZK
   light-client proof; the BLS work has to run as a program execution inside this pipeline.

Sequencer side (`lee/state_machine/src/validated_state_diff/mod.rs`):

1. The transaction needs **a private action or a recovery binding** (line 332). It may carry at
   most 8 private actions (`MAX_PRIVATE_ACTIONS`).
2. Duplicate checks, nonces, signatures, image IDs rebuilt from the live deployment, proof check.
3. Commitments are checked new and nullifiers unused.
4. Messages that crossed to public actors run on-chain. **This is metered** (fixed in v0.4; in
   v0.3 it ran up to 32M cycles unmetered). The budget is whichever is smaller: the block's
   remaining exec gas, or what the reserve pays for (`lez/chain_state/src/apply.rs`). The cycles
   are added to the block's exec gas.

Fees:

- Exec: flat `PRIVATE_VERIFY_GAS` = 409,764, plus the metered public part.
- Storage: flat `PROOF_BYTES + PRIVATE_PAD_BYTES` = 224,063.
- Both constants are marked `FIXME: Provisional`.
- About **4 private transactions fit per block** by storage, and about 24 by exec.

### Proving costs

| Case | Prove time | Source |
|---|---:|---|
| 131k-cycle program, standalone | 13.7 s | measured (LEZ v0.3) |
| 1M-cycle program, standalone | 112–126 s | measured (LEZ v0.3) |
| auth_transfer in PPE | 61.5 s | measured (LEZ v0.3) |
| Each extra chained call | ≈ 53 s | measured (LEZ v0.3) |
| Fixed private-transaction overhead | ≈ 233 s | measured (prototype) |
| Verify | ≈ 12 ms | measured (LEZ v0.3) |
| Proof size | 223,551 B, constant | both |

## 4. Light-client update: two paths

Verifier work per update: **12.53M cycles** for a real mainnet update with a handoff, 11.39M
for a finality-only update (measured, accelerated `blst`). That is over the 10M
per-transaction and per-block cap. Where it went on the prototype (12.77M): pairing 61%, hash
to G2 9%, the two committee SSZ roots 8.5% each, input decode 6%, signature decode 4%, the
rest 2%. See [results.md](results.md).

### 4a. Public split path (the fallback)

- The same verifier runs as public actor messages, split so each step stays under 10M. With
  12.5M cycles that is 2 transactions in 2 blocks. On the prototype (measured): prepare (every
  check but the pairing) 4.93M, then pairing 7.85M, with 741 bytes carried between them.
- On v0.4 each step is a message to a job actor that holds the intermediate result between
  steps.
- Anyone can submit with no prover, only a consensus endpoint and LEZ gas.

### 4b. Private-proven path (our proposal)

```
operator box   fetch update + 512 keys (Nimbus verified proxy)
               private actor (submitter's account, LC program) handles Update:     🔒 proven
                 committee root, finality / next-committee / execution branches, full BLS
                 → send_sync(instance, Advance { prev, next })
boundary       Advance is published                                                 📢
sequencer      verify proof (flat 409,764)
               public instance actor handles Advance (~15k cycles, metered):       🌐
                 assert from_own_program()
                 assert pre_state == prev        (no stale or racing update)
                 set_state(next); write header record once
```

| | Prototype, M3 Max (measured) | Current verifier, reference laptop, on the node (measured) |
|---|---|---|
| Cycles in the proven part | 12,903,333 | 12.53M (handoff) |
| Public part | 14,768 cycles | 32,905–56,998 cycles (with the record write) |
| Proving time | **995 s** (16.6 min) | **2,137–2,200 s** (35.6–36.7 min) |
| Peak RAM | 14.1 GB | 8.4–8.7 GB |
| Transaction size | 227,451 B | — |
| Exec gas charged | 419,311 (409,764 + 9,547) = 4.19% of a block | 442,669–466,762 |
| Storage gas charged | 224,063 = 22.4% of a block | 224,063 |
| Racing loser charged | 5,470,616 | — |

Design requirements (from reviewing the first spike):

1. **Bind the config.** A submitter that supplies `ChainConfig`, with `Advance` checking only
   the store bytes, so a forged `min_participants = 1` or fork schedule would pass. The instance
   must hold checkpoint and config (with `prev` covering all of it), or the instance ID must be
   `H(checkpoint ‖ cfg)`.
2. **Deploy immutable.** `from_own_program` compares addresses only. An upgradable program can be
   replaced, which breaks VS.3. The spike used `new_unchecked`.
3. **Still to build:** header records with a read handler, a freshness check through
   `timestamp_window`, and the consumer's reply handling.
4. Every operator needs a private account, because of the private-action rule. That is a
   documented submitter requirement.

| | Public split (4a) | Private-proven (4b) |
|---|---|---|
| Transactions per update | 2, in 2 blocks | 1 |
| Chain exec gas | 11.4–12.5M | 442,669–466,762 |
| Chain storage gas | the 52 KB update material as instruction data, plus job state | 224,063 |
| Submitter needs | consensus endpoint + gas | + a prover (35.6–36.7 min on M3 Pro 18 GB, CPU, on the node) + a private account |
| Lost race | wasted gas (≤ 1 step) | 5.47M gas charged and a wasted proof |
| Depends on provisional fees | no | yes (`PRIVATE_VERIFY_GAS`) |

Our answer to the RFP's central feasibility question:

1. 4a, measured on v0.4: it does not fit one transaction or one block, but fits two.
2. Offer 4b as the operating mode that needs no precompile.
3. Keep 4a as the permissionless fallback.

## 5. Attestation: two paths

Cost per kind of fact, as a whole public attestation (P.1, measured): a storage slot plus its
account 301k exec gas (3% of 10M), an account 180k, a receipt log 83k; each extra condition
about 1.5k.

### 5a. Public

The consumer's actor sends the query to the LC program's attest handler. Everything runs on the
sequencer and is visible.

### 5b. Private

The leak to avoid (Functionality #8) is a question of **which messages cross to public
actors, and what they contain**. Sending the query to a public actor would publish it.

Our design (idea B):

- The consumer and the attest logic both run as private actors, so the MPT proof and
  predicates are proven work and never published.
- The only crossing is one read of the public header record. It reveals which recorded header
  was used, nothing about the query.
- If replay protection uses a public statement-ID record, writing it reveals the statement ID.
  That links to an Ethereum event, which is already public, but not to the consuming account.
- How the read works on v0.4 is in section 6a. In short: it works within one transaction, the
  sender can be blinded, and the reply is checked against live public state.

Prove time, measured on the node: **14.2–15.2 min and 7.3–7.8 GB peak** on the reference
laptop (P.4). The projection from the ≈ 233 s fixed overhead had been 4–5 min.

## 6. Code reads (`952bd6dd`)

Read in the code, not yet run. Each is confirmed by a test in the issue it feeds.

### 6a. A private actor reading a public actor (feeds #7)

`lee/state_machine/core/src/execution_state.rs`:

- When a private actor sends to a public actor, the proven part records a `PrivateToPublic`
  boundary step. That step is published.
- The public actor runs on the sequencer. Its reply into the private actor is a
  `PublicToPrivate` step. The prover predicts it while proving, and the sequencer compares it
  with the live delivery. A mismatch fails with `CrossMessageMismatch`. So a private read is
  checked against public state at inclusion, not at proving time.
- Commit `f05c02ca` requires every predicted message to target a **private** actor. A prediction
  aimed at a public actor is refused (`UndeclaredActor`). Our reply goes to the private attest
  actor, so it is unaffected.
- What the boundary publishes: the destination actor, the message bytes, the sender, and only
  the non-private inherited authorizations (`disclose`).
- The sender is presented canonically or **blinded**: `AccountId::blinded(factor)` gives a
  fresh alias (`SenderPresentation::Blinded`).

What this means for private attestation:

- The outgoing message carries only the header-record ID. The query, proof and predicate stay
  in the proven part.
- The attest actor presents a blinded sender, so the consumer's account isn't published.
- The reply is the record's contents, which are public anyway.
- The proof commits to the record's state as predicted. Header records are written once, so
  the prediction can't go stale between proving and inclusion (R.11).

### 6b. Holding and paying out the native token (feeds soft #5)

`lee/state_machine/core/src/native_token.rs`:

- Balances are native-token actors: `Transfer { to, amount }`, `Credit`, `ReadState`.
- A program holds funds in a PDA it controls. It pays out with
  `custody_transfer(pda, seed, to, amount)`, a sync `Transfer` carrying the PDA seed as
  authorization. The fee program does exactly this for its fee inbox (`lez/programs/fee`).
- The recipient is credited by an **async** message, so in a later transaction.
- Anyone can fund the PDA with an ordinary `Transfer`.

So the staleness bounty works with existing primitives:

- The pool is a PDA of the light-client program, seeded by the instance ID.
- `Advance` pays the recipient named in the update.
- The open question is the private attestation's fee into the pool. That is one more boundary
  crossing, and it publishes an amount. A flat fee keeps the amount uninformative.

### 6c. Changing a deployed program (feeds VS.3, #2)

`lez/programs/program_loader/core/src/lib.rs`:

- A header is the loader's actor state, and an actor writes only its own state. Only loader
  messages can change it.
- The loader has three messages:
  - `WriteSegment` and `CreateHeader` require an empty target, so segments and headers are
    never overwritten.
  - `UpdateHeader` asserts `!old_header.immutable`.
- So within the protocol, an immutable program can't be changed. A protocol upgrade of LEZ
  itself is outside this guarantee, and the trust document has to say so.

### 6d. Private transactions from a client

`lez/wallet/src/lib.rs`:

- `send_privacy_preserving_tx(accounts, root, message, programs)` proves locally and submits
  over RPC. It picks a private fee payer if none is given (`auto_fee_payer`).
- `programs` is a `ProgramCatalog` (`AccountId → Program`), so a client can pass our deployed
  program's ELF.
- No separate submit path is needed for the node-level run.

## 7. Numbers still needed

Measured so far is in [results.md](results.md); what's left is in [bench-plan.md](bench-plan.md).

## 8. What changed from the v0.3 notes

Still true:

- The light-client trust state lives on LEZ; updates are verified once and recorded, and attest
  trusts the records without re-verifying signatures.
- The proven update avoids the 10M cap.
- Private attestation must cross into public as little as possible.
- The fee constants and the proving pipeline.

Changed:

- Plan/apply mechanics and the "plan can't read state" constraint are gone.
- The privacy question is now about boundary-crossing messages (section 5b).
- The 32M unmetered public part is closed; the public part is metered.
- The private-action rule now also accepts a recovery binding, and caps actions at 8.
- Immutability is a requirement, because programs are upgradable by default.
- The 28–34 min proving projection is replaced by measured times (section 4b).
- v0.3 plan/apply programs do not port to v0.4 as a thin shell.
