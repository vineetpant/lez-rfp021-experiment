# Claims, statement IDs and nullifiers

How a consumer program asks a question about Ethereum, how `eth_lc` answers it, what identifies
the answer, and how a consumer uses that identifier to act on an Ethereum fact exactly once.
This is the design we implement; the work still to do is tracked in the issues listed at the
end. RFP-021 items: F.3–F.8, F.15, S.4, U.6, VS.2, VS.3, soft #2 and #4.

## The flow, step by step

| # | Step | Status | Where |
|---|---|---|---|
| 1 | The LEZ program implements the light-client signature check and the surrounding checks in one function | ✅ built | `core::update` (`lc/types/src/core.rs`), run by the `Update` message |
| 2 | That function is proven privately on the operator's machine, which submits the transaction with the proof (the RISC Zero receipt) | ✅ built, on the node with real proofs | the operator's wallet proves `Update`; the sequencer verifies the receipt |
| 3 | The transaction advances the finalized header, stored as a finalised header in its own program-derived account | ✅ built | `Advance`, then `WriteRecord` at `for_public_pda(eth_lc, H("header" ‖ instance ‖ block_number))` |
| 4 | Consumers fetch from Ethereum what the predicate needs, e.g. `eth_getProof(address, slot, block)` for an ERC-20 balance (the state-trie and storage-trie paths) | ✅ storage proofs from `eth_getProof`, recorded in `tests/data`; logs from `eth_getBlockReceipts`, in the tests; live fetching is the operator daemon's and the client's (#29) | `src/chain.rs` (`storage_proof`); `tests/support` (receipt proofs); a balance slot is `keccak(holder ‖ balances_position)` |
| 5 | Consumers submit the proof and the predicate, e.g. a storage proof with `StorageValue equal to value`, to the program | ✅ built: a claim of one or more facts, each a proof with its conditions; logs on the node still to wire (#34) | `Attest` to the record account; the record fixes the block, and an older block is reached with an ancestry, not a block number |
| 6 | The program checks the proof, evaluates the predicate, and returns a statement with an ID | ✅ built | `statement::check`; the ID is derived from the proven Ethereum state (section 3) |

## 1. The question: a claim

A consumer program builds a **claim** in its own transaction (F.4, F.7): facts to prove against
one finalised header, each a Merkle-Patricia proof with the **conditions** its values must meet,
all of which must hold (soft #4). The module needs no change for a new question. These are the
proposal's types (section 4.5–4.6).

```rust
Claim { chain_id, facts: [Fact] }          // the chain the consumer expects (F.5); all facts must hold
Fact  { proof: Proof, conditions: [Condition] }

Proof = Log     { tx_index, receipt, proof, log_index }                          // under receipts_root
      | Storage { address, account, account_proof, slot, value, storage_proof }  // under state_root
      | Account { address, account, account_proof }                              // under state_root

Condition { subject, op, rhs: [u8; 32] }   // uint, int, address or bytes32, as a 32-byte word
Subject = AccountBalance | AccountNonce | AccountCodeHash
        | StorageValue { offset, width }   // a packed field; { 0, 32 } is the whole slot
        | LogEmitter | LogTopic(i) | LogDataWord(i) | LogTopicCount | LogDataLength
Op      = Equal | NotEqual | Less | AtMost | Greater | AtLeast
```

A condition on a value its fact doesn't establish (an account balance asked of a storage proof,
a topic the log doesn't have, a packed field that runs off the slot) is refused as
`MalformedInput`, never read as zero. Account conditions need an `Account` proof, so their ID
covers the account (section 3).

A claim may also carry an **ancestry**: an EIP-2935 proof or a parent-hash chain from a header
the instance holds back to an older block, so the facts are checked under that block's roots
(proposal section 8).

### What `eth_lc` checks

1. The finalised header is held by the instance the consumer names; with an ancestry, the
   history proof leads from it to the target block, whose header hashes to the proven block hash.
2. `chain_id` equals the header's (`WrongChain` otherwise).
3. Each fact's Merkle-Patricia path verifies against the target block's roots:
   - **Log:** from `receipts_root`, key `RLP(tx_index)`, to the receipt; the receipt's status
     must be success (a reverted transaction's logs prove nothing); the log at `log_index`.
   - **Storage:** from `state_root`, key `keccak(address)`, to the account; then from its
     `storage_root`, key `keccak(slot)`, to the value.
   - **Account:** the first of those two paths.
4. Each condition reads its subject from its fact and compares. A subject the fact doesn't carry
   is refused (`MalformedInput`), never read as zero.

Any failure is a typed refusal (U.5), and the whole transaction reverts.

### Which fact to use

| | Log | Storage | Account |
|---|---|---|---|
| Proves | an event happened | a slot holds a value at that block | an account's balance, nonce, code |
| Fits | deposits, transfers, any emitted event | a write-once mapping, configuration | balances, contract existence |
| ID class | **occurrence**: one per event | **state** | **state** |

Deposits use logs.

- **ERC-20:** the token contract itself emits `Transfer(address indexed from, address indexed to,
  uint256 value)` on every transfer. Emitter = token, topic 1 = sender, topic 2 = recipient,
  data word 0 = amount.
- **Native ETH:** a plain ETH transfer emits no log, so deposits go to a vault contract whose
  `receive()` emits `Deposit(address indexed sender, uint256 amount, uint256 nonce)`.
  `msg.sender` is the real sender, including Safe, ERC-4337 and EIP-7702 accounts. No
  transaction-trie parsing and no signature recovery sit in the trusted path.

A storage proof alone can't say who sent ETH: the vault's balance shows a total, not a sender.

### Example: a 10 ETH deposit

On Ethereum a user sends 10 ETH to the vault; `receive()` emits
`Deposit(sender = 0xUSER, amount = 10e18, nonce = 42)` in transaction 57 of block 26,400,000.
The LEZ bridge program builds:

```rust
Claim {
    chain_id: 1,
    facts: [Fact {
        proof: Log { tx_index: 57, receipt, proof, log_index: 0 },
        conditions: [
            { subject: LogEmitter,     op: Equal, rhs: word(VAULT) },
            { subject: LogTopic(0),    op: Equal, rhs: keccak256("Deposit(address,uint256,uint256)") },
            { subject: LogDataWord(0), op: Equal, rhs: word(10e18) },   // fixed denomination
            // optional: { subject: LogTopic(1), op: Equal, rhs: word(0xUSER) }
        ],
    }],
}
```

With an EIP-2935 ancestry from the latest finalised header when block 26,400,000 isn't one the
instance holds.
`Equal` rather than `AtLeast` keeps deposits in fixed denominations, which RFP-022 recommends so
that deposits look alike.

An ERC-20 deposit is the same with the token's `Transfer` log: emitter = token, topic 0 =
`keccak256("Transfer(address,address,uint256)")`, topic 2 = vault, data word 0 = amount.

## 2. The answer: a statement

```rust
Statement {
    instance,               // the instance whose header was used (F.15)
    chain_id,               // the chain it is about (F.5)
    block_number,           // the block the facts were proven under
    facts: [ProvenFact { class, id, conditions }],  // one per fact, in the claim's order
}
```

The consumer receives it in the same transaction, on both paths (F.7):

- **Public:** `Attest { instance, claim, ancestry, reply: true }` to the finalised header's account;
  `eth_lc` replies `Attested { statement }` to the caller and emits it as an event.
- **Private:** the consumer's private actor calls `eth_lc`'s private actor, which checks the claim
  inside the proof, confirms the header with one `CheckRecord { instance, header }` to the
  header's account, and returns the statement to the consumer inside the proof. Only
  `CheckRecord` is published by `eth_lc`.

## 3. Statement IDs (F.6)

Each fact's ID is a hash of the Ethereum state it proves, and of nothing else:

```
id = SHA-256( "eth_lc/statement/v1" ‖ chain_id ‖ kind ‖ fields )
```

| Kind | Tag | Fields |
|---|---|---|
| storage | 0 | `address ‖ slot ‖ value` |
| log | 1 | `block_hash ‖ tx_index ‖ log_index ‖ emitter ‖ n_topics ‖ topics ‖ len(data) ‖ data` |
| account | 2 | `address ‖ nonce ‖ balance ‖ code_hash` |

Integers are 8-byte little-endian, words and hashes 32 bytes, addresses 20 bytes; `n_topics`
and `len(data)` are 8-byte little-endian lengths.

### Every field comes from verified data

For the 10 ETH deposit:

| Field | Where it comes from | Why the caller can't choose it |
|---|---|---|
| `chain_id` | the finalised header | checked against the claim; the header was written by the program |
| `block_hash` | the deposit block's hash | in the finalised header, verified by the light client, or proven from it through EIP-2935 or the parent-hash chain |
| `tx_index` | the receipts-trie key | a different index is a different path, and the proof fails |
| `log_index`, emitter, topics, data | the proven receipt | read from the receipt the path leads to |

### What the ID leaves out, and why

| Left out | Why |
|---|---|
| The conditions | Two consumers asking different questions about one deposit get the same ID, so they can't derive incompatible identifiers for one event (RFP, "Uniqueness is the consumer's job"). |
| The header used | One deposit can be attested against its own block's header or against a newer header through history. If the header were in the ID, one deposit would have several IDs and could be consumed once per header. |
| The instance | An instance and its replacement (F.10) give the same ID for the same deposit, so a migration can't be used to consume a deposit twice. |
| The path | Public, private and the dry run run the same `statement::check`. |
| The proof bytes | Two valid proofs of one receipt are the same fact. |

### What the ID keeps, and why

| Kept | Why |
|---|---|
| `chain_id` | A Sepolia deposit never shares an ID with a mainnet one at the same vault address (F.5). |
| `block_hash ‖ tx_index ‖ log_index` | One ID per occurrence: two identical 10 ETH deposits from one sender are two IDs. |
| The kind tag | A log ID never equals a storage or account ID. |
| The versioned domain | A later encoding never collides with `v1`. |

### The properties

- **Deterministic:** given one deposit, every consumer, every header, every instance of that
  chain and both paths compute the same 32 bytes. Identical Ethereum state, identical ID.
- **Collision-resistant:** the encoding is injective (fixed widths, length prefixes, a kind
  tag), so two distinct facts differ in at least one hashed byte, and SHA-256 keeps their IDs
  apart. Distinct state, distinct ID.
- **Published:** the encoding is part of the IDL (U.4), with test vectors, so a consumer in any
  language derives the same ID.

### The two classes

- **Occurrence** (logs): an event happens once, so its ID is safe as a one-shot nullifier.
- **State** (storage, account): the same state is the same ID at any block, as F.6 requires, so
  the same value at two different times gives the same ID. Safe as a one-shot nullifier only
  when the state is written once: a vault's `deposits[nonce]` mapping that is never overwritten
  qualifies; a balance does not.

The class travels in the statement, so a consumer can refuse a state ID where it needs a
one-shot nullifier.

## 4. Nullifiers: acting exactly once

Uniqueness is the consumer's job (RFP-021, out of scope for the module); the module supplies the
ID and this guidance (S.4). A consumer that must act once per Ethereum fact keeps a nullifier
keyed on the statement ID.

### The nullifier account

**Assumptions.** The consumer program owns its nullifiers: `eth_lc` returns the statement ID
and stops there (RFP-021, out of scope). The reference consumers (U.6) and the Recommended
Consumer Pattern (S.4) implement the scheme below, and other consumers are expected to follow
it.

**A program-derived address (PDA)** is an account address computed from a program's ID and a
seed, `for_public_pda(program, seed)`, rather than from a key pair. No one holds a private key
for it, so only that program can write to it, and anyone can compute it from the same two
inputs. `eth_lc`'s finalised-header accounts already work this way, at `PDA(eth_lc, H("header" ‖ instance ‖ block_number))`.

A consumer's nullifier for a statement ID is the account at:

```
nullifier(id) = for_public_pda(consumer_program, H("nullifier" ‖ id))
```

- `consumer_program`: the account belongs to the consumer, so only it can mark the ID spent.
- `id`: one address per Ethereum fact, the same address every time for the same fact.
- `"nullifier"`: a label that keeps these addresses apart from any other account the consumer
  derives from an ID.

The consumer handles it with one message to that account:

```
Spend { id }  → refuses if the account already holds a value, else writes it once
```

For the 10 ETH deposit with ID `0xab12…`:

```
addr = for_public_pda(bridge, H("nullifier" ‖ 0xab12…))   always the same for this deposit

first mint:   the account at addr is empty    → write it, mint 10 wETH
second mint:  the account at addr is written  → refuse; the whole transaction reverts
```

Like a locker numbered by the deposit's ID that only the bridge holds the key to: empty means
unused, filled means spent.

- One account per ID, found by derivation, so checking it costs the same however many deposits
  were consumed (P.9).
- Derived from the consumer program, so two different consumers of one deposit (a mint and a
  read-only escrow check) never block each other; each guards its own action.
- Write-once, the same pattern as `eth_lc`'s finalised-header accounts, so a spent ID stays spent.

### The consumer's order of operations

All in one transaction, so a failure anywhere reverts everything and spends nothing (R.2):

1. Send the claim and receive the statement (section 2).
2. Check `statement.instance` and `statement.chain_id` equal the consumer's own fixed
   configuration (F.5, F.15).
3. Check the fact's class is **occurrence**, or **state** only for a slot the consumer knows is
   written once.
4. `Spend { id }` on the nullifier account.
5. Act: mint, release, record.

If an attestation can't be produced, the consumer refuses the action; it never falls back to an
unverified input (S.4).

### Public consumers

The nullifier account is public: everyone sees which deposit was consumed and by which
transaction. That is the public path's nature.

### Private consumers

The check for "already spent" must be against public state at inclusion; otherwise two private
transactions could each consume the same deposit. So the nullifier crosses the boundary:

```
user's private account (proven on their machine):
  consumer program
    └─ eth_lc private actor: checks it         ── CheckRecord { instance, header } ─▶ header account
    ◀─ statement
    └─ Spend { id }  ──────────────────────────────────────────────────────────────▶ nullifier account (write-once)
    └─ mint to a private account
```

- **Published:** `CheckRecord` (which header was used), `Spend { id }` (which deposit was
  consumed), and the fee reserve, each from a one-time alias.
- **Not published:** the claim, the sender, the conditions, the consuming account, the LEZ
  recipient.
- **Which deposit was consumed** is already public on Ethereum, so publishing the ID adds no new
  correlation signal (F.8). Hashing it again would hide nothing: anyone can hash every deposit.
  What stays hidden is **who on LEZ** received it. Fixed denominations make deposits look alike,
  so the nullifier doesn't single out a recipient (RFP-022's anonymity set).
- **Races:** two private transactions spending the same ID both prove, but the second `Spend`
  finds the account written and reverts. Transactions spending different IDs touch different
  accounts and land independently in the same block (R.11).

### Compound claims and batches

A claim over several facts (soft #4) or a batch (soft #2) returns one ID per fact. The consumer
spends the ID of the fact it consumes, for example the deposit log in a claim that also checks a
balance. It never keys a nullifier on a combination: a different combination over the same
deposit would get a different key and could be consumed again.

### Consumers that don't need a nullifier

A read-only check, such as an escrow condition evaluated many times, skips steps 3 and 4. The
RFP's example: a bridge mint is once per deposit, an escrow check may be evaluated repeatedly.

### Migration

IDs don't depend on the instance, so nullifiers carry across an instance replacement (F.10): a
deposit consumed through the old instance stays spent when the consumer moves to the new one.
The consumer pins the module's program and the instance in its own immutable code or config,
never through a `ProgramId` in mutable state (VS.3).

## 5. Status

| Piece | Today | Tracked in |
|---|---|---|
| Claims of several facts, each with conditions on: slot values and packed fields, account balance, nonce and code hash, log emitter, topics, data words, topic count and data length | built, tested natively; storage on the node | — |
| Account proofs, length-prefixed log encoding, class and instance in the statement, one ID per proof, ID tests | built, tested natively (`tests/statement.rs`) | #31 |
| Log attestations on the node | native only | #34 |
| Signed comparisons, the mapping-slot helper, the IDL with the ID test vectors | to build | #13 |
| The private path returning the statement to a consumer program | to build | #39 |
| The vault, the ERC-20 and native-ETH consumers with nullifiers | to build | #36 |
| The Recommended Consumer Pattern in the docs | this file is its basis | #37 |
| Cross-chain refusal on the node | to build | #35 |
