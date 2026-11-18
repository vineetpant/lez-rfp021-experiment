### RFP ID

RFP-022 — Privacy-Preserving Wrapped ERC-20 and Ether Bridge for LEZ

### Your Project Name

Pier

### Team or Organization Name

Congruence Labs

### Primary Contact

TODO (team): email, Telegram, Discord or Signal

### Team Members

Member 1
Name or pseudonym: Chirag Mahaveer Parmar
Social links: https://github.com/chirag-parmar
Role: TODO (team)
Status: TODO (team)

Member 2
Name or pseudonym: Shoaib Ahmed
Social links: https://github.com/hu55a1n1
Role: TODO (team)
Status: TODO (team)

Member 3
Name or pseudonym: Vineet Pant
Social links: https://github.com/vineetpant
Role: TODO (team)
Status: TODO (team)

Member 4
Name or pseudonym: Manas Nagaraj
Social links: https://github.com/hopinheimer
Role: TODO (team)
Status: TODO (team)

### Project Summary

A pier is the column a bridge stands on. It rises from the caisson sunk in the riverbed and carries the span across. **Pier** is the bridge that stands on **Caisson**, our RFP-021 attestation primitive, and carries ERC-20s and native ETH between Ethereum and LEZ in both directions. All code will be released under the MIT and Apache-2.0 licences.

An immutable Ethereum vault escrows deposits in fixed denominations. A LEZ bridge program mints the wrapped token, backed one to one, once a claim proves the deposit through Caisson against finalised Ethereum. Burning on LEZ entitles the holder to release the original from the vault. Neither leg names the other chain's destination, and nothing but amount, token and timing links a deposit to its mint or a burn to its release.

- **No bridge secret, and nothing leaves the wallet.** A deposit belongs to its destination LEZ account, and only that account can claim it; LEZ's own circuit proves the sender. The random values that open a deposit or a burn are kept on-chain, encrypted to that account like any LEZ private note. There is no seed export and no new key, a leaked value can't steal anything, and hardware wallets work as far as LEZ supports them.
- **Inbound rests on Caisson.** The vault writes each deposit's commitment once, to its own storage slot. A claim proves that slot privately as a Caisson Storage fact and keys its nullifier on the fact's statement ID, blinded with a random value.
- **The user never builds a proof for Ethereum.** On the way back, a private Redeem on LEZ breaks the link between a burn and its ticket, so what reaches Ethereum carries no secret, anyone can prove it, and one Groth16 check covers a whole batch of releases.
- **Accountable signers.** LEZ v0.4 has no state root and no validity proof, so the outbound leg needs signers; we propose LEZ's own sequencer threshold, the trust Bedrock already uses for withdrawals. Every signer posts a bond, a false state is provable on LEZ and burns it, and one honest signer can veto a false state on Ethereum during a candidate window.
- **Gas for first-time users.** A private transaction pays its own gas, from the prover's account. A deposit can optionally buy fuel: native LEZ delivered by any operator and repaid from ETH already escrowed.
- **A failsafe that needs no admin**, scoped to what a fault stranded, and still working if the fault is in Caisson.
- **Private by default, public when chosen.** A fully public path in both directions needs no proving at all. On the private path, one action per direction starts a background job, and proving runs during finality and privacy waits that are longer than it.

Why it matters: LEZ's private DeFi needs assets whose value is already established, and users hold those on Ethereum. If moving them takes a trusted custodian, or leaves a public trail from an Ethereum address to a LEZ account, LEZ's privacy ends where the funds enter.

### Technical Approach

Requirement IDs refer to RFP-022 (F, U, R, P, S, Sec, Priv, Soft). Figures are tagged **measured** (from a run we watched), **estimated** (from measured parts or fixed protocol costs, method given) or ⟨measure⟩ (an M0 or M1 deliverable). Source references are LEZ `v0.4.0-rc1` and Bedrock `d480efc`.

#### 0. Where RFP-022 and LEZ v0.4 meet a gap

RFP-022 asks the vault to release on cryptographic verification of a LEZ burn, with no trusted signers or federations. LEZ v0.4, checked in its source, has no state root in its block header (`lez/common/src/block.rs`: block id, previous hash, timestamp and a hash of the whole block) and no validity proof of its blocks. Bedrock relies on the zone's sequencer threshold for withdrawals from a zone (`ChannelWithdraw`; two thirds of the `sequencer_stake` committee). Verifying Bedrock's leadership proofs on Ethereum would also need an epoch nonce and two ledger roots that no header carries (`zk/proofs/pol/src/inputs.rs`). So the outbound leg depends on a choice about how Ethereum learns LEZ state. The same question arises for RFP-023 (draft), which inherits RFP-022's approach.

- **What we propose.** LEZ's own sequencer threshold signs the bridge state: the trust Bedrock already applies to withdrawals, and so the security of the LEZ network as v0.4 defines it. We add the accountability v0.4 doesn't provide: bonds, fault proofs checked on LEZ, and a minority veto on Ethereum (§5).
- **What we'd like to agree with Logos in M0.** (1) Whether the sequencer threshold meets F7 and the trust model on v0.4. (2) Whether sequencers would run our signing module, with a bridge bond. (3) Logos's direction on a state commitment in the LEZ header and validity proofs of LEZ blocks.
- **Other paths, for Logos to choose from.** *(a) A bonded attestor set:* anyone may sign by posting a bond, under the same accountability. It resembles an attestor federation, which the trust model aims to avoid, and we state its weaknesses openly: attestors as a class could stall releases while LEZ runs, and admission takes the current set's signatures. *(b) The strict path, with no signers:* Ethereum verifies Bedrock finality and LEZ validity itself. It matches the trust model fully; how much of it lands in this grant depends on what Logos provides and on what the early milestones measure, so it is planned with a decision point (§5).

#### 1. Components

- **Vault** (Ethereum, immutable, no proxy): escrows registered ERC-20s and native ETH in fixed denominations; one write-once slot per deposit; verifies signed LEZ state and pays releases.
- **Bridge program** (LEZ, immutable): claims, mints, burns, Redeems, tickets, checkpoints, nullifiers, caps, fees, bonds and fault proofs, per configuration. It pins one Caisson program and instance.
- **Caisson** (LEZ, our RFP-021 primitive): finalised Ethereum headers and the Storage fact a claim proves. Read only.
- **Off-chain, all Logos modules with headless CLIs**: bridge core module, mini-app and admin UI, and the relayer, fuel-operator, signer and batch-prover daemons.

**Configurations (F15, F16).** A configuration lives at `PDA(bridge, config_id)`, with `config_id = keccak(chain_id ‖ vault ‖ lez_chain_id ‖ zone_id ‖ bridge_program ‖ caisson_program ‖ caisson_instance ‖ admin_authority)`. Every commitment, nullifier, ticket and signed message binds `config_id`, so nothing valid under one pairing or configuration is accepted under another. Mutable parameters (registry, caps, fees, depth) sit under it, writable only by its own admin. A client lists configurations from their events and switches by `config_id`. Each configuration pairs one EVM chain with one zone, and each chain's token gets its own wrapped mint, so an incident on one chain stays contained to that chain's supply.

#### 2. Deposit (F1, F2, F8, F17)

```
dest  = the LEZ account that will receive the mint and own the deposit (private by default)
ρ     = 32 random bytes
C     = keccak("bridge/deposit/v1" ‖ config_id ‖ token ‖ denom ‖ dest ‖ ρ)
memo  = EncryptedNote to dest's viewing public key: (token, denom, dest, ρ, fuel opening)

vault.deposit(token, denom, C, memo)
  require registered(token, denom), not frozen, deposits[C] == 0
  pull exactly denom            // SafeERC20 and a balance-difference check (USDT); msg.value == denom for ETH
  deposits[C] = keccak("bridge/asset/v1" ‖ token ‖ denom)     // written once, at slot keccak(C ‖ DEPOSITS_SLOT)
  total_in[token] += denom
  emit Deposit(C, token, denom, memo)
```

- **No LEZ destination on Ethereum.** The call, the event and the state hold only `C`, token, denomination and a fixed-length memo. `C` hides `dest` (without `ρ` nobody can test a guess), and the memo uses LEZ's own note encryption (ML-KEM-768), built from `dest`'s public keys alone. Its ciphertexts are assumed not to reveal their recipient, as LEZ's private notes are.
- **The vault records what it received**, and the claim checks the opening against it, so a commitment can't claim more than was deposited.
- **Any Ethereum wallet signs the deposit.** It sees only opaque bytes and holds no bridge secret.
- **Fixed denominations (F8)**, for example 100 / 1,000 / 10,000 USDC, USDT and DAI and 0.1 / 1 / 10 ETH and WETH. Each (token, denomination) is its own anonymity set; final sets are sized from bridge volume.
- **Gas, estimated:** about 80–95k for ETH and 110–135k for an ERC-20, including the encrypted memo (about 1.1 KB). ⟨measure⟩ in Foundry.

#### 3. Claim on LEZ (F3, F4, F12, R1, R3, R4)

The private claim (the default) is a private transaction **sent from `dest`**, so LEZ's circuit proves control of `dest`, as it does for every private transfer. Inside the proof:

1. A Caisson Storage fact: slot `keccak(C ‖ DEPOSITS_SLOT)` of the vault holds `keccak("bridge/asset/v1" ‖ token ‖ denom)` at block `T`, against a finalised Caisson record `N` (through EIP-2935 or a parent-hash chain when `T` is older).
2. The opening of `C` with `ρ`, read from the memo.
3. `T ≤ N − depth`.
4. `nf = keccak("bridge/nf/v1" ‖ config_id ‖ statement_ID ‖ ρ)`.

Published: `CheckRecord(N)` (the only Caisson message), `Spend{nf}` at a write-once PDA, the cap counter, and the mint of the denomination minus a fixed fee to `dest`.

- **F3, met as written.** Uniqueness is keyed on the statement identifier the attestation carries. Each deposit has its own write-once slot, so its own statement ID, which Caisson's nullifier-safety class marks safe as a one-shot key. Blinding it with `ρ` keeps it from naming the deposit at every mint (Priv #1).
- **Theft-proof (Sec #3).** `dest` is sealed in `C`, and only `dest` can send the claim; learning `ρ` gives no power to claim.
- **Atomic and retry-safe.** A rejected claim changes nothing. Caisson records are immutable, so a valid claim never expires (R4).
- **Concurrency, verified in source.** A private transaction's public part runs on live state at inclusion and must reproduce what was proven (`validated_state_diff/mod.rs:374–392`). The bridge sends nothing back into the private part, so claims in one block don't conflict.

**Modes, both directions (F4, F6, Priv #7):**

| Direction | Mode | What links the two ends | User proving | When |
|---|---|---|---|---|
| In | Private claim → private account | Amount, token, timing only | One LEZ private transaction | Default |
| In | Private claim → public account | The same; the balance is visible | One LEZ private transaction | Explicit choice |
| In | Public claim → public account | Fully linked | None: a signed public transaction (P1) | Explicit override |
| Out | Burn, then a private Redeem by the hidden redeemer | Amount, token, timing only | The Redeem, plus the burn if from a private account | Default |
| Out | Public burn with a direct ticket | Fully linked; `eth_dest` hidden until release | None: a signed public transaction | Explicit override |

**Finality (F12).** Recommended inbound default: Ethereum finality, read through a Caisson record. Residual risk: a captured sync committee could sign a forged header (the assumption RFP-021 accepts; at least 10,944 ETH of committee stake, not slashed today). The next committee's record exposes it within about 27 h and freezes claims (EV1), so the mint cap per 27-hour window is set to the loss a deployer accepts. Stricter setting: the two-committee rule, up to about 27 h. Outbound default: Bedrock finality (k = 120, about 1 h on testnet) plus a candidate window of at least 1 h. Every past depth is kept, so a change never invalidates a valid claim.

**Who pays (fuel).** On v0.4 a private transaction pays its fee inside its own proof, from a private account the prover owns (`lez/wallet/src/account_manager.rs:755`); a relayer can submit but can't pay. A deposit can optionally buy fuel: `f` in ETH for a fixed amount of native LEZ. Soon after the deposit, any operator sends a sealed credit to a blinded alias of `dest`, with the alias's opening and recovery binding carried in the memo, and is repaid in wrapped ETH backed by the escrowed `f`. Verified in source: credits can go to a blinded alias (`native_token.rs:66–78`, `program/mod.rs:248`); spending one publishes a nullifier that needs the account's secret key (`circuit_io.rs:246`); and every private action is published in one uniform shape, padded to 7 (`circuit_io.rs:470`). So a fuelled claim looks like any other, and optional fuel doesn't split the anonymity set. Sized at about 10.5M native units for a claim and a Redeem, from the measured 5,232,864 per private transaction at the floor fee.

#### 4. Burn, Redeem, Release (F5, F6, F7, R2)

```
Burn (LEZ), from a private or public account
  redeemer = the account that may Redeem, by default a private account of the user's;  t = 32 random bytes
  B    = keccak("bridge/burn/v1" ‖ config_id ‖ token ‖ denom ‖ redeemer ‖ t)
  memo = EncryptedNote to redeemer: (token, denom, redeemer, t)
  burn denom; insert B into the burn tree; burn_root[n] written once

Redeem (LEZ, later), a private transaction sent from redeemer
  prove B ∈ burn_root[n];  Spend{ nf' = keccak("bridge/burn-nf/v1" ‖ config_id ‖ t) }
  K = keccak("bridge/ticket/v1" ‖ config_id ‖ token ‖ denom ‖ eth_dest ‖ r)   (r random) into the ticket tree
  memo = EncryptedNote to redeemer: (eth_dest, r);  total_redeemed[token] += denom

Release (Ethereum), by the user or any relayer
  require ticket_root[block_id] finalised, registered, not frozen, within the release cap
  MerklePath(K) → ticket_root[block_id];  require !spent[K];  spent[K] = true
  require total_released[token] + denom ≤ redeemed[token]           // inline supply check (EV4)
  pay denom − fixed relayer fee to eth_dest
```

- **The burn names no Ethereum destination (F5)**, and it hides who may redeem. Any account can burn; only `redeemer` can Redeem, so a burn from a public account stays unlinkable once redeemed, and learning `t` gives no power.
- **Public option, direct ticket** (explicit override): a public burn may write `K` at once, with no Redeem and no proving. `eth_dest` stays hidden until release; the burn and the release are linked.
- **Who proves what.** The user proves only ordinary LEZ private transactions (claim, Redeem, and a burn from a private account). The Groth16 proof Ethereum checks wraps public signatures, so anyone can make it, and one proof covers a batch. No x86-only step reaches the user's machine. Each release costs a Merkle path, about 60–90k gas estimated.
- **Cost.** The private way back asks the user for one more LEZ private transaction, at LEZ's usual cost (about 15 min on a CPU laptop), in return for no secret-bearing proof on Ethereum, no x86 step for the user, and cheap releases. It runs in the background (§9). We considered unlinking at release with a small user proof wrapped by a relayer instead, and didn't choose it as the default: it sends a secret-bearing proof to Ethereum, adds a Groth16 verification to every release, and running both paths would split the outbound anonymity set.

#### 5. How Ethereum learns LEZ state (F7)

```
Checkpoint{b}       anyone; a public transaction declaring block_window(b..b+1)
  state_at[b] = H(ticket_root ‖ total_redeemed[·] ‖ freeze_flags)    (write-once)

Signed message      msg = H("lez/bridge-state/v1" ‖ zone_id ‖ bridge_program ‖ config_id ‖ block_id ‖ state)
                    signed by a threshold t of the key set, only for checkpointed blocks final on Bedrock

Vault               attest(state, proof)      Groth16 of the threshold check (RISC Zero guest, audited verifier, pinned)
                                              → candidate[block_id]
                    dispute(block_id, sig)    any one signer's secp256k1 veto, within the window
                    reconfirm(block_id, proof) n − 1 of n reinstate a vetoed candidate
                    finalize(block_id)        after the window: ticket_root[block_id] usable

LEZ                 Fault{msg, sig, signer}            msg's state ≠ state_at[block_id] → burn the signer's bonds
                    FalseDispute{dispute, sig, signer} vetoed state = state_at[block_id] → burn the vetoer's bonds
```

- **Why bonds.** LEZ's `sequencer_stake` can't slash on evidence: a slash "carries no proof the offence happened" and needs a two-thirds vote (`lez/programs/sequencer_stake/core/src/lib.rs:312–321`). So each signer bonds wrapped tokens per registered token, with bond ≥ release cap over the release delay ÷ t. A false state costs its signers at least what it could release, and burning the bonds takes that value back out of the wrapped supply, restoring the backing. A newly registered token's releases finalise only once enough signers hold bonds in it.
- **Candidate window and minority veto.** Ethereum can't tell a false state from a true one, so each signed state waits. During the window one honest signer stops a false state with no admin; a veto of a true state is provable on LEZ and slashed; two conflicting signed states freeze releases (EV5). If every signer colludes, the release cap bounds the loss and the bonds restore it.
- **The key set changes; the rule doesn't (Sec #8).** The vault accepts a state signed by a threshold of the key set it tracks, and a new key set only through a handoff signed by a threshold of the current one. That rule is fixed at deployment, as Caisson follows sync-committee handoffs under a fixed rule.
- **Path (a)** uses the same message, vault, bonds and fault proofs with a different key set; anyone may join by bonding.
- **Path (b)** removes the signers: a Bedrock light client on Ethereum (leadership proofs are BN254 Groth16, checkable with Ethereum's precompiles, once the header carries the epoch nonce and ledger roots or the ledger is proven) and a RISC Zero proof of LEZ block execution committing to the ticket root. Under it, inbound and the LEZ side of outbound ship complete, a validity-path milestone measures the prover per block and ends in a decision point with Logos, and the vault is deployed once its release verifier is final (or deployed without release first, with a successor adding it, per §7).

#### 6. Privacy (Priv #1–#7, F8)

The linkage test of a round trip (M3) checks unlinkability on public data.

| Signal | Why it remains | What an observer learns | Mitigation, on by default (U5) |
|---|---|---|---|
| Amount | Ethereum transfers are public | The (token, denomination) set | Fixed denominations; fees fixed per denomination |
| Timing | Transactions are timestamped | Ordering within the set | Finality wait plus a suggested delay; claims never expire |
| Record used | A claim names a finalised record | "Deposited before N" | The proof hides which slot |
| Burn root used | A Redeem names a root | "Burned before n" | Every earlier burn stays in the set |
| Fee payer | Someone pays gas | Nothing: paid inside the proof | Fuel for first-time users; padding |
| Fuel | Delivered in the open | That a deposit bought fuel | Unlinkable to the claim |
| Split amounts | Several deposits close together | A cluster | Independent random delays per claim; a warning |
| Relayer's view | It submits | A finished proof, its time, the user's IP | Switchable per operation; IP is out of scope and disclosed |
| Public mode | The user chose it | The exact deposit or burn | Explicit confirmation |

**Anonymity set:** for a claim against record `N`, the unclaimed deposits of the same token and denomination before `N − depth`; for a release, the unredeemed burns of the same token and denomination before the root the Redeem used. The mini-app shows the set size before the user acts. A failed transaction reveals nothing new: the block records only its index among reverted transactions (`lez/common/src/block.rs:62`), and only the fee action settles (Priv #5). No component but the user's own wallet sees anything that links the legs (Priv #4): the Ethereum wallet sees opaque bytes, relayers a finished proof, the fuel operator an alias already public, provers public signatures.

#### 7. Keys, recovery, registry, caps, fees, freeze, failsafe

- **Ownership and recovery (F9, R5).** A position belongs to a LEZ account: a deposit to `dest`, a burn to its `redeemer`. Recovery: restore the wallet from its seed (which regenerates its accounts), trial-decrypt the vault's and the bridge's fixed-length memos with the accounts' viewing keys (a view tag speeds the scan), then check nullifiers and `spent[K]` and finish what is open. That is the construction the RFP prefers: claim data regenerated from on-chain events the user's own keys can scan for. A dedicated bridge account's viewing key discloses only bridge activity (Soft #2).
- **Why not seed-derived secrets.** Bridges of this kind commonly derive a bridge key from the seed. We rejected it: the LEZ wallet gives modules only public keys (`core_ffi`), so the seed or a derived key would have to leave the wallet; its secrets would be bearer secrets, so a leak could steal unredeemed burns; and it fails with hardware wallets, since Keycard keeps the seed on the card (`load_lee_key`) and supports public accounts only today. With account-bound ownership, public claims and burns sign on the card now, and private ones follow when LEZ extends the card to private accounts, with no change to Pier.
- **Registry (F10, F17).** Per token: address, wrapped mint (LP-0013, ATA-compatible), decimals, denominations, caps. USDC and USDT (6 decimals), DAI and WETH (18), and native ETH as its own entry. The wrapped mint keeps the token's decimals, so registration refuses anything not exactly representable. Delisting stops deposits only; a removed denomination stays claimable and burnable; a new one starts an empty anonymity set.
- **Caps (F11, Sec #4).** Rolling mint caps on LEZ and release caps on the vault, per token and global. The mint window is about 27 h (how long the next sync committee's record takes to expose a forged header); the release window covers the candidate window and release delay. Values: TODO (team). A configuration may raise a cap for a Caisson record backed by restaked stake, but only after its challenge window, and never requires backing.
- **Fees (F13).** In the wrapped token, through the RFP-001 fee switch, mint and burn set independently. Initial rate 0 on testnet; recommended production start 10 bps; range 0–100 bps, with the ceiling fixed per configuration at deployment. Each rate is rounded to a fixed amount per (token, denomination), so it can't fingerprint; the mint fee is taken at claim and the burn fee at burn, so amounts stay on the grid. Routing: treasury, relayer, and optionally Caisson's staleness bounty, so bridge traffic pays for fresh records. The fuel price `f` is separate, and not a fee.
- **Freeze (F14, Sec #5).** The RFP-002 freeze authority pauses minting or redemption, globally or per token, on each half independently. A Caisson freeze stops claims by itself. Evidence freezes need no admin: anyone proves that Caisson froze the instance (EV1), that more was minted than received (EV2), that a backed record was proven false (EV3), or that the signers signed two conflicting states (EV5). EV4 is checked inline in every release; EV6 (a false state) and EV7 (a veto) are in §5. The freeze works at token and protocol level only, as a consequence of the privacy design.
- **Immutability and migration (Sec #8).** Both halves are immutable, with no proxy, and Caisson is pinned in `config_id`. A new version is a new pair; old wrapped tokens keep redeeming from the old vault, and no escrow moves between vaults. Adopting value commitments (Soft #1) is such a new version, or, if built in from the start behind a switch, needs no redeployment (a team decision).
- **Failsafe (Sec #9).** A refund is decided on LEZ, where the nullifiers live, and paid on Ethereum against the vault's own record. `Refund` is sent from `dest`, opens `C`, writes the same nullifier a claim would (so a deposit is never both claimed and refunded) and issues `K_r = keccak("bridge/refund/v1" ‖ config_id ‖ C ‖ eth_addr)`; the vault pays it against `deposits[C]`. It needs no Caisson, so it works when the fault is in Caisson. It acts only on what the fault stranded: a freeze names its scope (the configuration or one token), and a refund must fall inside it. Refunds count against the release cap. If the fault is in the outbound verification itself, the failsafe is stop-only: escrow stays locked and no admin can move it.

#### 8. Security

**Trust model (Sec #7).** Ethereum and LEZ consensus; the sync committee through Caisson (bounded by depth and caps); the token issuer (blacklisting the vault affects every holder); the signers (§5). Off-chain roles are liveness only: relayers, Caisson operators, the fuel operator and the batch prover can each be replaced by anyone, the user included. If every signer stops, releases wait and tickets stay valid; under the proposed design, that is LEZ itself stopping.

**Bridge exploits, mapped to the RFP's appendix.**

| Pattern (incidents in the RFP's appendix) | Here |
|---|---|
| A small signer set's keys stolen (Ronin, Harmony Horizon, Multichain, Orbit Chain) | Inbound has no keys. Outbound signers are bonded, a false state is provable on LEZ, one honest signer can veto, and the release cap bounds what a captured threshold can take |
| Signature check bypassed (Wormhole) | `verify_strict` in a RISC Zero guest; every input bound into the journal; RISC Zero's audited verifier, pinned, not its router |
| Initialisation or upgrade error (Nomad; Ronin 2024; a Wormhole near-miss) | No upgrade path, no proxy; a release against an unattested root is refused; key-set changes only through the signed handoff |
| Proof verification bug (BNB Token Hub) | Caisson's Merkle-Patricia checks with tamper tests; mint caps; EV2 freezes when supply exceeds what was received |
| Deposit accounting error (Qubit, Meter.io, THORChain's router) | Exact pull with a balance-difference check; `msg.value == denom`; the slot records the asset received |
| Generic cross-chain call abused (Poly Network) | No generic messaging; nothing received can change keys, registry or verification |

**Verification plan (Sec #2, #6).** Stateful invariant fuzzing of the vault (wrapped supply ≤ escrow; one release per ticket; one refund per deposit; no release on an unfinalised root; caps hold); symbolic checks of `release`, `refund`, `finalize`, `dispute` and the handoff; property tests of the guest logic; and an adversarial suite: forged slot values, wrong vault or slot, other chains and configurations, wrong openings, replayed nullifiers and releases, forged, missing and replayed signatures, handoffs below threshold, refund after claim and claim after refund.

#### 9. Modules and usability (U1–U10)

- Bridge core module (deposit, claim, burn, release, recover, refund, registry read and admin), on the LEZ module, the Logos Ethereum core module with its verified proxy, and the wallet SDK. It holds no secret.
- **One action per direction (U5).** "Bridge in" and "Withdraw to 0x…" each start a background job: it proves the claim once the Caisson record exists and submits after the privacy delay; it burns, proves the Redeem, submits, and a relayer releases. A job resumes after a restart, and an interrupted one has submitted nothing (R7). The app shows an arrival time, not a proof time.
- **Proofs made early stay valid.** LEZ accepts any commitment root its tree has ever had (`root_history`, `lee/state_machine/src/state/mod.rs:623–626`), and records and burn roots are written once, so proving overlaps the waits.
- Mini-app (deposit and redemption end to end, recovery, registry with caps and utilisation, a leak indicator, private pre-selected) and admin UI; daemons as Logos modules with headless CLIs and a documented operator journey (install, configure, run, monitor) (U3); events through LP-0012; a hand-written SPEL IDL with a drift check (U4); errors with one code per failure mode, revealing no deposit or burn (U8); switchable endpoints (U9); our own module catalog with the release action (U10, S3).
- **Common Ethereum wallets (Soft #5).** Basecamp prepares the whole deposit and the user signs it in their own wallet through WalletConnect or a QR code; every bridge secret stays in Basecamp, and an ERC-7730 descriptor gives Ledger clear signing. We rejected deriving secrets from a wallet signature (phishable, and fails for smart-contract wallets).

#### 10. Performance (P1–P7)

| Item | Figure | Class |
|---|---|---|
| Complete private storage attestation on a LEZ v0.4 node (the core of a private claim) | 14.6 min, 7.3 GB, charged 5,232,864 native units (Apple M3 Pro, 18 GB, CPU) | measured |
| Private receipt verification, every private transaction | 409,764 exec gas + 224,063 storage gas | measured |
| A deposit older than the record (history inside the private proof) | 5,238,752 cycles whatever the distance; no extra charge | measured |
| Public claim: storage check | 270,062 cycles; a public attestation charged 2,469,032 on the node | measured |
| Caisson tracking, shared by every consumer | About 11.2M native units a sync-committee period | measured, derived |
| Deposit gas | ETH about 80–95k; ERC-20 about 110–135k | estimated |
| Release gas | About 60–90k (Merkle path, no proof) | estimated |
| `attest`, once per batch | One Groth16 verification and one slot | ⟨measure⟩ |
| Latency in | Ethereum finality (13–19 min) + a Caisson record (none if older than the latest; else about 48–56 min) + the privacy delay; the claim proof overlaps them | measured parts |
| Latency out | Privacy delay (the Redeem proof overlaps it) + Bedrock finality (about 1 h) + candidate window (at least 1 h) | estimated |
| Growth | Ethereum: one slot per deposit and per release; LEZ: one PDA per nullifier and per checkpoint | estimated |

Declared minimum 16 GB, CPU proving; M0 measures the claim and the Redeem on a mid-range laptop and on the minimum, Linux and macOS (P4). GPU proving cut Caisson's update proof from 1,107 s to 30 s (measured); laptop GPU support is ⟨verify⟩.

#### 11. Dependencies

- **RFP-021: Caisson**, our proposal, as the base. Pier needs from any RFP-021 primitive only a finalised storage-slot statement with a deterministic, instance-independent statement ID, private consumption that publishes only a record reference, synchronous use in one transaction, and a readable freeze; another primitive with those is consumed through one adapter.
- **RFP-001** (admin, fee switch), **RFP-002** (freeze), **LP-0013** (token authorities, ATA-compatible accounts), **LP-0012** (events), the **Logos Ethereum core module** with its verified proxy, the **wallet SDK**, **RISC Zero** with its audited Groth16 verifier.

#### 12. Risks and what we ask in M0

| Risk or question | Handling |
|---|---|
| The outbound trust path | Logos chooses in M0 week 1: sequencers, path (a) or path (b) (§0) |
| Sequencers signing and bonding | Part of the same decision |
| Account authorisation for a program call | Confirm a bridge program can require a claim, Redeem or Refund to be sent from an authorised account (`program/mod.rs:379`) |
| Bridge memos | Confirm the wallet returns the user's decrypted bridge memos, or the memos use LEZ's note format so its scan finds them; no secret leaves the wallet |
| Fuel authorisation path | Test that the operator's signature authorises the transfer inside the bridge's call; fallback: a top-level transfer in the same transaction |
| Fee reserved in an early proof | Check the wallet's headroom against base-fee rises; re-prove at submission if needed |
| User proving takes minutes | LEZ's cost for any private transaction; background proving; public path needs none |
| Testnet 0.4 not live | Develop on a local v0.4 devnet; deploy when live |
| Small anonymity sets at launch | The set-size indicator; denominations sized from volume |
| Token issuers | Can blacklist the vault; documented |

#### 13. Requirement coverage

Coverage: F1–F2, F17 (§2); F3, F4, F12 (§3); F5–F7 (§4, §5); F8 (§2, §6); F9 (§7); F10–F16 (§1, §7); U1–U10 (§9); R1–R7 (§3, §4, §9); R8 (RISC Zero and its audited verifier; Caisson's reuse plan); R9 (§8); R10 (a round trip in CI against a standalone sequencer and a Kurtosis Ethereum devnet with a consensus layer); R11–R14 (a requirement-to-test matrix and the docs, M4); P1–P7 (§10); S1 (testnet deployment, M4); S2–S3 (§9); S4 (a decision log, opened in M0); Sec #1 (audit milestone); Sec #2–#9 (§5, §7, §8); Priv #1–#7 (§3, §6); Soft #1–#5 (§7, §9).

### Milestones, Payout and Timeline

The build totals about 17 weeks, inside the RFP's 16–20, plus the audit. The audit is TODO (team): within this budget, or procured separately. Shared work with our RFP-021 proposal, if both are funded, is delivered once and deducted from this one: TODO (team).

**Milestone 0 — Feasibility and decisions**
Payout: TODO
Duration: 1 week
Deliverables: the outbound path decided with Logos in week 1; the account-authorisation and fuel checks; the full private claim and Redeem measured on a mid-range laptop and the declared minimum, Linux and macOS; signature cycles in the guest; bond sizes, window length and fuel price proposed; decision log opened.

**Milestone 1 — Vault and inbound**
Payout: TODO
Duration: 4 weeks
Deliverables: the vault (write-once deposit slots, fuel escrow, registry for USDC, USDT, DAI, WETH and native ETH, caps, freeze); private and public claims through Caisson, with nullifiers; the `Fuel` transaction; adversarial inbound tests and invariant fuzzing of the vault. Developed against a Caisson devnet instance until Caisson is on testnet.

**Milestone 2 — Outbound**
Payout: TODO
Duration: 5 weeks
Deliverables: burn and ticket trees, the private Redeem, the direct-ticket public burn, `Checkpoint`, signer bonds, `Fault` and `FalseDispute` on LEZ; key-set tracking, `attest`, `dispute`, `reconfirm`, `finalize` and `release` on Ethereum, with release caps and the candidate window; symbolic checks of the release path. Under path (b), this milestone carries the validity-path prototype and its decision point instead.

**Milestone 3 — Recovery, failsafe and privacy**
Payout: TODO
Duration: 3 weeks
Deliverables: recovery from the wallet seed by trial-decrypting the memos, including fuel and unreleased tickets; the scoped `Refund` failsafe and a migration rehearsal; viewing keys; the linkage test and full-diff test of a round trip; anonymity-set reporting; split-amount spacing.

**Milestone 4 — Modules, testnet and release**
Payout: TODO
Duration: 4 weeks
Deliverables: core module, mini-app and admin UI; relayer, fuel-operator, signer and prover daemons with headless CLIs; the one-action background flows; the calldata handoff for common wallets; SPEL IDL; module catalog; deployment on a public Ethereum testnet and LEZ testnet 0.4; e2e CI; user, developer, operator and deployer docs; the trust document.

**Milestone 5 — Audit**
Payout: TODO
Duration: TODO
Deliverables: third-party audit of the vault and the bridge program, remediation of findings, published report.

### Total Requested Budget (USD)

TODO (team): the sum of the milestone payouts above.

### Relevant Experience

- **Chirag Mahaveer Parmar** builds the Nimbus verified proxy in nimbus-eth1, which the Logos Ethereum core module wraps: 68 merged pull requests, among them its REST-based light client, light-client scoring across endpoints, EIP-2935 anchor jumps and BLOCKHASH, Gloas updates, and its C, Go and WASM bindings. He fixed `eth_getProof` in nim-web3 and was an Ethereum Protocol Fellow (cohort five). The bridge's Ethereum side runs through that module, and its deposit proofs are `eth_getProof` storage proofs.
- **Shoaib Ahmed** worked on Cosmos IBC at Informal Systems, where each chain verifies the other's consensus with an on-chain light client. In ibc-rs he implemented the light-client extraction (ADR 011). He was lead developer of Quartz, a TEE framework whose enclaves verify the chain through a light client, and of Cycles, with arkworks settlement circuits and nullifier derivation; he now works on Shielded Labs' Zeronym. He did the research and the experiment behind Caisson that proved a mainnet sync-committee update as one private LEZ v0.4 transaction.
- **Vineet Pant** worked on decentralized and self-sovereign identity at evan.network (vade-evan, didcomm-rs): verifiable credentials that reveal only chosen attributes. As an Ethereum Protocol Fellow (cohort six) he worked on EIP-7745 (the trustless log index) in Nimbus, used for a proof of concept, and he has contributed to open-source Ethereum projects, among them Reth, Ream and Colibri. He built the `eth_lc` prototype whose measured private attestations ground this proposal's claim costs, and checked this design against the LEZ v0.4 source.
- **Manas Nagaraj** is a developer at Sigma Prime on Lighthouse, the Rust Ethereum consensus client, and a former Ethereum Protocol Fellow. His merged Lighthouse work includes Gloas support and fork-choice updates, and he has contributed gossipsub 1.2 to rust-libp2p.

TODO (team): any Solidity and audit-preparation experience, for the vault.

### Post-Delivery Plan

TODO (team): owner and durations. Proposed shape:

- **Security reports:** first response within TODO business days; for high severity, a fix or a published mitigation within TODO days, shipped as a new version with migration notes, since verification is immutable.
- **LEZ releases:** benchmarks and the e2e suite re-run on each LEZ tag within TODO weeks; a migration plan before any change the deployed version can't follow.
- **Ethereum forks:** tracked through Caisson; bridge-side changes published TODO weeks before activation.
- **Testnet services for TODO months:** a relayer, a fuel operator, a batch prover and, under path (a), an attestor, each with a public uptime log.
- **Integrations:** answers to the DEX, lending and stablecoin teams that consume wrapped assets.

### Permissions and Consent

- [ ] I/We confirm Logos may contact me using the primary contact information provided above for follow-ups and next steps.
- [ ] I/We consent to Logos using information from this proposal publicly such as blogs case studies social posts or analytical reporting. Redactions can be requested at any time.

### Program Requirements

- [ ] I/We have read and agree to the Logos RFP Terms and Conditions and I understand that no Grant is awarded and no right to payment arises unless a Grant Agreement is executed.
- [ ] I/We understand that RFP specifications are proposals rather than instructions, that Logos makes no representations as to their legal or regulatory treatment, and that we are responsible for assessing what we build, deploy or operate and for complying with the laws that apply to us.
- [ ] I/We understand this project must be open-sourced under the MIT and Apache 2.0 Licenses unless explicitly approved otherwise.
- [ ] I/We are prepared to deliver milestone-based outcomes.
