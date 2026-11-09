//! Operator and consumer client for the `eth_lc` light-client program.
//!
//! Usage:
//!   eth_lc deploy <eth_lc.bin> <payer>
//!   eth_lc init <program> <instance> <chain-dir> <payer>
//!   eth_lc update <eth_lc.bin> <program> <instance> <chain-dir> <operator> <payer>
//!   eth_lc attest-public <program> <instance> <getproof.json> <payer> [case]
//!   eth_lc attest-private <eth_lc.bin> <program> <instance> <getproof.json> <operator> <payer> [case]
//!   eth_lc show <program> <instance>
//!   eth_lc pending <program> <instance> <chain-dir>
//!   eth_lc published <program> <instance> <block> <payer>
//!   eth_lc check <chain-dir>
//!
//! `chain-dir` holds Beacon API JSON for one mainnet chain segment: `genesis.json`,
//! `fork_schedule.json`, one `bootstrap-<slot>.json` with its `.root`, one `updates-*.json`
//! array, and optionally `finality_update.json`. `scripts/fetch-chain.sh` writes one from a
//! live beacon node; `tests/data/mainnet-1904` is a recorded one. Every update is checked with
//! Helios's consensus checks, natively first, so a proof is never spent on one that fails.
//!
//! `instance` is a public account the wallet owns; `Init` must be signed by it. `operator`
//! is a private account the wallet owns, whose actor runs the proven checks. Private fees are
//! paid from any funded private account the wallet owns, and the surplus is refunded to that
//! payer's blinded alias, the wallet's default, so no public account is linked to the step.
//! The refund arrives as a pending credit, which the next private send claims; its amount is
//! printed as a `FEE …-refund` line.
//!
//! After each private step a `PUBLISHED` line lists every public actor and every message that
//! crossed the private/public boundary, and a `PRIVACY` line says whether the included
//! transaction, proof and all, contains any private account the wallet owns, `payer`, or
//! (for an attestation) the queried address, slot or proof leaves.
//!
//! An attestation `case` is `honest` (the default) or a negative test, which must fail:
//! `tampered-proof` flips one byte of the storage proof, `false-claim` asserts the slot is zero
//! when it isn't, and `unheld-record` (private only) proves against a header the instance
//! doesn't hold.
//!
//! Three cases attest to a block older than the latest record, the one `getproof-<n>.json`
//! is at: `history` reaches it through the EIP-2935 history contract, `history-chain` through
//! the parent-hash chain of headers, and `history-forged`, which must fail, through EIP-2935
//! with one byte of the target's header changed. `scripts/fetch-chain.sh`'s `history` and
//! `header` modes fetch what they need into the same directory. Each attestation prints an `ATTEST` line with its outcome:
//!
//! - `accepted`: included and not reverted, the only outcome that means the statement holds;
//! - `reverted`: included and charged, but its effects undone because a check failed;
//! - `unprovable`: the wallet couldn't build the transaction, because the check fails when the
//!   wallet runs it locally, so no proof exists;
//! - `rejected`: the sequencer refused it or never included it.
//!
//! Every charge is printed as a `FEE` line, every proven step's wall time as a `TIME` line,
//! and every transaction's outcome as a `STATUS` line.

use std::{collections::BTreeMap, time::Instant};

use common::transaction::LeeTransaction;
use eth_lc_types::history::{Ancestry, history_slot, target_record};
use eth_lc_types::statement::{self, Cmp, Field, Predicate, Proof, Query};
use eth_lc_types::oracle::{self, AnyUpdate};
use eth_lc_types::{HeaderRecord, Instance, LcMsg, RecordState, Store, core, record_account, record_of, record_seed};
use lee::{
    AccountId, Actor, BoundaryStep, native_token,
    privacy_preserving_transaction::circuit::ProgramCatalog, program::Program,
};
use lez_hello_world::chain::{Chain, hex32, hex_decode, load_chain, storage_proof};
use lez_hello_world::{deploy_program, fees, follow_private_tx};
use sequencer_service_rpc::{RpcClient as _, SequencerClient};
use wallet::{AccountIdentity, AccountMention, WalletCore};

/// The recorded vectors and `fetch-chain.sh` are both Ethereum mainnet.
const CHAIN_ID: u64 = 1;

const USAGE: &str = "usage:
  eth_lc deploy <eth_lc.bin> <payer>
  eth_lc init <program> <instance> <chain-dir> <payer>
  eth_lc update <eth_lc.bin> <program> <instance> <chain-dir> <operator> <payer>
  eth_lc attest-public <program> <instance> <getproof.json> <payer> [honest|tampered-proof|false-claim]
  eth_lc attest-private <eth_lc.bin> <program> <instance> <getproof.json> <operator> <payer> [honest|tampered-proof|false-claim|unheld-record]
  eth_lc show <program> <instance>
  eth_lc pending <program> <instance> <chain-dir>
  eth_lc published <program> <instance> <block> <payer>
  eth_lc check <chain-dir>";

fn args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn id(s: &str) -> anyhow::Result<AccountId> {
    s.trim_start_matches("Public/")
        .trim_start_matches("Private/")
        .parse()
        .map_err(|e| anyhow::anyhow!("bad account id {s}: {e:?}"))
}

fn abs(path: &str) -> anyhow::Result<String> {
    Ok(std::fs::canonicalize(path)
        .map_err(|e| anyhow::anyhow!("{path}: {e}"))?
        .to_string_lossy()
        .into_owned())
}

/// The instance's state, read from the node.
async fn read_instance(
    client: &SequencerClient,
    program: AccountId,
    instance: AccountId,
) -> anyhow::Result<Instance> {
    let account = client
        .get_account_view(Actor::new(instance, program))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no account {instance}"))?;
    let state = account.data.actor_state(program);
    anyhow::ensure!(!state.as_ref().is_empty(), "{instance} holds no eth_lc instance");
    Ok(borsh::from_slice(state.as_ref())?)
}

/// `instance`'s record account of the header at `slot`, as a transaction mentions it. It is a
/// program-derived address, so a transaction that creates it carries the seed as evidence that
/// no one holds a key to it.
fn record_mention(program: AccountId, instance: AccountId, slot: u64) -> AccountMention {
    AccountIdentity::PublicPda { program, seed: record_seed(&instance, slot) }.select_program_actor_state(program)
}

/// `instance`'s record of the header at `slot`, read from its account, if written.
async fn read_record(
    client: &SequencerClient,
    program: AccountId,
    instance: AccountId,
    slot: u64,
) -> anyhow::Result<Option<RecordState>> {
    let account = client.get_account_view(Actor::new(record_account(&program, &instance, slot), program)).await?;
    let Some(account) = account else { return Ok(None) };
    let state = account.data.actor_state(program);
    if state.as_ref().is_empty() {
        return Ok(None);
    }
    Ok(Some(borsh::from_slice(state.as_ref())?))
}

/// An update that advances an instance, checked natively, with everything `Update` sends.
struct NextUpdate {
    update: core::Update,
    current: Vec<u8>,
    next: Option<Vec<u8>>,
    store: Store,
    record: Option<HeaderRecord>,
    handoff: bool,
}

/// The update in `chain-dir` that advances `held` furthest, verified natively with the same
/// checks the program runs, so a proof is never spent on an update that would be refused.
fn next_update(held: &Instance, dir: &str) -> anyhow::Result<NextUpdate> {
    let chain = load_chain(dir, CHAIN_ID)?;
    anyhow::ensure!(chain.cfg == held.cfg, "{dir} is not the chain this instance was bound to");
    next_update_in(&chain, &held.store)
}

fn next_update_in(chain: &Chain, store: &Store) -> anyhow::Result<NextUpdate> {
    let committee = |root: &[u8; 32]| chain.committee(root).map(oracle::committee);
    let current = committee(&store.current_committee).ok_or_else(|| anyhow::anyhow!("the segment lacks the current committee's keys"))?;
    let next = match &store.next_committee {
        Some(root) => Some(committee(root).ok_or_else(|| anyhow::anyhow!("the segment lacks the next committee's keys"))?),
        None => None,
    };
    let mut best: Option<NextUpdate> = None;
    for u in &chain.updates {
        let update = oracle::update(u);
        let Ok((after, advanced)) = core::update(&chain.cfg, store, &update, &current, next.as_deref()) else { continue };
        let record = if advanced { Some(record_of(&chain.cfg, &update.finalized, None, core::signers(&update.bits) as u16).map_err(|e| anyhow::anyhow!("{e:?}"))?) } else { None };
        let handoff = matches!(u, AnyUpdate::Update(_)) && after.next_committee != store.next_committee;
        if best.as_ref().is_none_or(|b| after.finalized.slot > b.store.finalized.slot) {
            best = Some(NextUpdate { update, current: current.clone(), next: next.clone(), store: after, record, handoff });
        }
    }
    best.ok_or_else(|| anyhow::anyhow!("nothing in the segment advances the instance past slot {}", store.finalized.slot))
}

/// An `eth_getProof` response as a statement that the slot holds a nonzero value.
/// The query an attestation sends: the slot is nonzero, by its `eth_getProof` proof. The
/// negative cases break it one way each.
fn storage_query(getproof: &str, case: &str) -> anyhow::Result<Query> {
    let mut proof = storage_proof(&abs(getproof)?)?;
    let mut cmp = Cmp::Gt;
    match case {
        "honest" | "unheld-record" | "history" | "history-chain" | "history-forged" => {}
        "tampered-proof" => {
            // The leaf's hash no longer matches its parent's reference to it.
            let leaf = proof.storage_proof.last_mut().ok_or_else(|| anyhow::anyhow!("empty storage proof"))?;
            *leaf.last_mut().ok_or_else(|| anyhow::anyhow!("empty proof node"))? ^= 1;
        }
        // The slot is nonzero, so "equals zero" is false.
        "false-claim" => cmp = Cmp::Eq,
        other => anyhow::bail!("unknown attestation case {other}\n{USAGE}"),
    }
    Ok(Query {
        chain_id: CHAIN_ID,
        proof: Proof::Storage(proof),
        predicates: vec![Predicate { field: Field::SlotValue, cmp, value: [0; 32] }],
    })
}

/// Prints the `ATTEST` line for an included attestation, from whether it reverted.
async fn report_attestation(client: &SequencerClient, step: &str, case: &str, tx: common::HashType, block: u64) -> anyhow::Result<()> {
    let reverted = fees::report_status(client, step, tx, block).await?;
    println!("ATTEST step={step} case={case} outcome={}", if reverted { "reverted" } else { "accepted" });
    Ok(())
}

/// The native credits waiting for this wallet's private accounts, by commitment position. A
/// private step's fee refund arrives as one, to the fee payer's alias.
async fn pending_credits<C: wallet::WalletSequencerClient>(wallet_core: &mut WalletCore<C>) -> anyhow::Result<BTreeMap<u64, u128>> {
    Ok(wallet_core
        .owned_pending_messages()
        .await?
        .iter()
        .filter_map(|m| m.native_credit().map(|amount| (m.position, amount)))
        .collect())
}

/// Prints a private step's refund `FEE` line. `charged` is minus the refund this step created;
/// `swept` is what its send claimed of earlier refunds into the fee payer's balance, so that the
/// payer's drop plus `swept` is the reserve.
fn report_refund(label: &str, before: &BTreeMap<u64, u128>, after: &BTreeMap<u64, u128>) {
    let refund: u128 = after.iter().filter(|(p, _)| !before.contains_key(p)).map(|(_, a)| a).sum();
    let swept: u128 = before.iter().filter(|(p, _)| !after.contains_key(p)).map(|(_, a)| a).sum();
    println!("FEE step={label} charged=-{refund} swept={swept}");
}

/// Lists what private transaction `tx` published, and checks that none of `secrets` is in it.
///
/// One `PUBLISHED` line per public actor and per message crossing the private/public boundary,
/// with what the message carries. Then the whole included transaction, proof and all, is
/// searched for each secret's bytes, and for every private account this wallet owns. The
/// `PRIVACY` line says `pass` only if none was found.
async fn report_published<C: wallet::WalletSequencerClient>(
    wallet_core: &WalletCore<C>,
    client: &SequencerClient,
    step: &str,
    (program, instance, record): (AccountId, AccountId, AccountId),
    tx: common::HashType,
    mut secrets: Vec<(String, Vec<u8>)>,
) -> anyhow::Result<bool> {
    let (included, _) = client.get_transaction(tx).await?.ok_or_else(|| anyhow::anyhow!("{tx} is not in a block"))?;
    let LeeTransaction::PrivacyPreserving(private) = &included else {
        anyhow::bail!("{tx} is not a private transaction");
    };
    let owned: Vec<AccountId> = wallet_core.storage().key_chain().private_account_ids().map(|(id, _)| id).collect();
    // Anything else is an alias or a system account: an ID no one can tie to an owner.
    let name = |id: AccountId| match id {
        _ if id == program => "eth_lc".to_owned(),
        _ if id == instance => "instance".to_owned(),
        _ if id == record => "record".to_owned(),
        _ if id == native_token::NATIVE_TOKEN_PROGRAM_ID => "native token".to_owned(),
        _ if owned.contains(&id) => format!("PRIVATE ACCOUNT {id}"),
        _ => format!("unlinked {id}"),
    };
    let actor = |a: &Actor| format!("{}@{}", name(a.account_id), name(a.program_account_id));
    let carries = |to: &Actor, message: &[u8]| {
        if to.program_account_id == program {
            borsh::from_slice::<LcMsg>(message).map_or_else(|_| format!("{} bytes", message.len()), |m| {
                let m = format!("{m:?}");
                m.split([' ', '{', '(']).next().unwrap_or_default().to_owned()
            })
        } else if to.program_account_id == native_token::NATIVE_TOKEN_PROGRAM_ID {
            borsh::from_slice::<native_token::Message>(message).map_or_else(|_| format!("{} bytes", message.len()), |m| format!("{m:?}"))
        } else {
            format!("{} bytes", message.len())
        }
    };
    let execution = &private.message.execution;
    for a in &private.message.public_context.actors {
        println!("PUBLISHED step={step} public-actor {}", actor(a));
    }
    for b in &execution.boundary {
        let (dir, d) = match b {
            BoundaryStep::PrivateToPublic(d) => ("private-to-public", d),
            BoundaryStep::PublicToPrivate(d) => ("public-to-private", d),
            BoundaryStep::EndPrivateSubtree | BoundaryStep::EndPublicSubtree => continue,
        };
        println!(
            "PUBLISHED step={step} {dir} from={} to={} carries={}",
            actor(&d.envelope.from),
            actor(&d.envelope.to),
            carries(&d.envelope.to, &d.envelope.message)
        );
    }
    if let Some(root) = &execution.public_root {
        println!("PUBLISHED step={step} public-root to={} carries={}", actor(&root.to), carries(&root.to, &root.message));
    }
    println!(
        "PUBLISHED step={step} sealed-messages={} recovery-bindings={} private-actions={} refund-to={} fee-fallback={}",
        execution.async_messages.len(),
        execution.recovery_bindings.len(),
        execution.private_actions.len(),
        // `None` is the payer's alias, inside the proof.
        execution.fee.as_ref().and_then(|f| f.refund_to).map_or_else(|| "payer-alias".to_owned(), name),
        execution.fee.as_ref().is_some_and(|f| f.fallback.is_some()),
    );

    let bytes = borsh::to_vec(&included)?;
    secrets.extend(owned.iter().map(|id| (format!("private account {id}"), id.as_ref().to_vec())));
    let found: Vec<String> = secrets
        .iter()
        .filter(|(_, s)| !s.is_empty() && bytes.windows(s.len()).any(|w| w == s.as_slice()))
        .map(|(n, _)| n.replace(' ', "_"))
        .collect();
    println!(
        "PRIVACY step={step} searched={} bytes={} {}",
        secrets.len(),
        bytes.len(),
        if found.is_empty() { "pass".to_owned() } else { format!("found={}", found.join(",")) }
    );
    Ok(found.is_empty())
}

/// What a storage attestation must keep private: the account, the slot, and the proof leaves,
/// which hold the account's state and the slot's value.
fn query_secrets(query: &Query) -> Vec<(String, Vec<u8>)> {
    let Proof::Storage(p) = &query.proof else { return vec![] };
    let mut out = vec![("queried address".to_owned(), p.address.to_vec()), ("queried slot".to_owned(), p.slot.to_vec())];
    out.extend(p.account_proof.last().map(|n| ("account leaf".to_owned(), n.clone())));
    out.extend(p.storage_proof.last().map(|n| ("storage leaf".to_owned(), n.clone())));
    out
}

/// The cases that must be refused.
fn is_negative(case: &str) -> bool {
    matches!(case, "tampered-proof" | "false-claim" | "unheld-record" | "history-forged")
}

/// Block `n`'s header RLP from `dir`, checked to hash to the block hash the node reported, so a
/// wrong encoding fails here rather than in a proof.
fn header_rlp(dir: &str, n: u64) -> anyhow::Result<Vec<u8>> {
    let rlp = hex_decode(&std::fs::read_to_string(format!("{dir}/header-{n}.rlp"))?)?;
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(format!("{dir}/header-{n}.json"))?)?;
    let hash = hex32(&j["result"]["hash"])?;
    anyhow::ensure!(alloy_primitives::keccak256(&rlp).0 == hash, "header {n}'s RLP doesn't hash to its block hash");
    Ok(rlp)
}

/// How a history case reaches the block `getproof` is at from `anchor`, or `None` for an
/// attestation against the anchor itself.
fn ancestry(getproof: &str, anchor: &RecordState, case: &str) -> anyhow::Result<Option<Ancestry>> {
    if !case.starts_with("history") {
        return Ok(None);
    }
    let path = abs(getproof)?;
    let dir = path.rsplit_once('/').map_or(".", |(d, _)| d).to_owned();
    let target: u64 = path
        .rsplit_once("getproof-")
        .and_then(|(_, f)| f.strip_suffix(".json"))
        .ok_or_else(|| anyhow::anyhow!("{getproof} isn't a getproof-<block>.json"))?
        .parse()?;
    let r = anchor.record.block_number;
    Ok(Some(match case {
        "history-chain" => Ancestry::ParentChain { headers: (target..=r).rev().map(|n| header_rlp(&dir, n)).collect::<Result<_, _>>()? },
        _ => {
            let mut header = header_rlp(&dir, target)?;
            if case == "history-forged" {
                // The real hash is in the history contract; this header no longer hashes to it.
                *header.last_mut().expect("a header") ^= 1;
            }
            let mut proof = storage_proof(&format!("{dir}/history-{r}-{target}.json"))?;
            // The slot is the target's by construction; nodes return the key shortened anyway.
            proof.slot = history_slot(target);
            Ancestry::Eip2935 { proof, header }
        }
    }))
}

/// What the publication check must not find for a history case: the target's header and the
/// history contract's slot for it.
fn ancestry_secrets(ancestry: &Option<Ancestry>) -> Vec<(String, Vec<u8>)> {
    match ancestry {
        Some(Ancestry::Eip2935 { proof, header }) => {
            vec![("target header".to_owned(), header.clone()), ("history slot".to_owned(), proof.slot.to_vec())]
        }
        Some(Ancestry::ParentChain { headers }) => headers.last().map(|h| vec![("target header".to_owned(), h.clone())]).unwrap_or_default(),
        None => vec![],
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = args();
    let arg = |n: usize| -> anyhow::Result<&str> {
        a.get(n).map(String::as_str).ok_or_else(|| anyhow::anyhow!("missing argument {n}\n{USAGE}"))
    };
    let mut wallet_core = WalletCore::from_env()?;
    let client = wallet_core.client_owned();

    match arg(0)? {
        "deploy" => {
            let payer = id(arg(2)?)?;
            let bytecode = std::fs::read(arg(1)?)?;
            let before = fees::snapshot(&client, payer).await?;
            let program = deploy_program(&mut wallet_core, bytecode, payer).await?;
            fees::report_step("lc-deploy", before, fees::snapshot(&client, payer).await?);
            println!("deployed at header account {program}");
        }

        "init" => {
            let (program, instance, dir, payer) = (id(arg(1)?)?, id(arg(2)?)?, abs(arg(3)?)?, id(arg(4)?)?);
            let chain = load_chain(&dir, CHAIN_ID)?;
            let bootstrap = oracle::bootstrap(&chain.bootstrap);
            let committee = oracle::committee(chain.bootstrap.current_sync_committee());
            core::bootstrap(&chain.cfg, &chain.checkpoint, &bootstrap, &committee).map_err(|e| anyhow::anyhow!("the bootstrap doesn't verify: {e:?}"))?;
            let checkpoint_slot = bootstrap.header.beacon.slot;
            let message = LcMsg::Init { cfg: chain.cfg, checkpoint: chain.checkpoint, bootstrap, committee, rlp: None };
            // Init checks the bootstrap publicly: the committee's SSZ root over 512 keys, decoded
            // from CBOR. Its gas limit is raised to the per-transaction cap so a heavy bootstrap
            // isn't cut off; only Init gets this, because a refused public transaction is charged
            // its whole limit.
            let mut config = wallet_core.config().clone();
            config.gas_limit = 10_000_000;
            wallet_core.set_config(config);
            // `Public`, not `PublicNoSign`: the instance account signs, which Init requires.
            let account = AccountIdentity::Public(instance).select_program_actor_state(program);
            let accounts = vec![account, record_mention(program, instance, checkpoint_slot)];
            let before = fees::snapshot(&client, payer).await?;
            let tx = wallet_core
                .send_pub_tx_paid_by(accounts, 0, Program::serialize_message(message)?, Some(payer))
                .await
                .map_err(|e| anyhow::anyhow!("Init failed: {e:?}"))?;
            let block = fees::wait_included(&client, tx).await?;
            let after = fees::snapshot(&client, payer).await?;
            fees::report_public_tx(&client, "lc-init", tx, before, after).await?;
            fees::report_status(&client, "lc-init", tx, block).await?;
            let held = read_instance(&client, program, instance).await?;
            println!("initialised at slot {} block {}", held.store.finalized.slot, held.latest.block_number);
        }

        "update" => {
            let bytecode = std::fs::read(arg(1)?)?;
            let (program, instance, dir) = (id(arg(2)?)?, id(arg(3)?)?, abs(arg(4)?)?);
            let (operator, payer) = (id(arg(5)?)?, id(arg(6)?)?);
            let held = read_instance(&client, program, instance).await?;
            let next = next_update(&held, &dir)?;
            println!(
                "update: slot {} -> {} (block {}), handoff {}, signers {}",
                held.store.finalized.slot,
                next.store.finalized.slot,
                next.record.as_ref().map_or(held.latest.block_number, |r| r.block_number),
                next.handoff,
                next.record.as_ref().map_or(0, |r| r.signers),
            );
            let advances = next.record.is_some();
            let next_slot = next.store.finalized.slot;
            let message = LcMsg::Update {
                instance: Actor::new(instance, program),
                cfg: held.cfg,
                store: held.store,
                update: next.update,
                current: next.current,
                next: next.next,
                rlp: None,
            };
            let programs = ProgramCatalog::from([(program, Program::new(bytecode.into())?)]);
            let mut accounts = vec![
                AccountIdentity::PrivateOwned(operator).select_program_actor_state(program),
                AccountIdentity::PublicNoSign(instance).select_program_actor_state(program),
            ];
            // `Advance` writes the record of the newly finalized header to its own account.
            if advances {
                accounts.push(record_mention(program, instance, next_slot));
            }
            let credits = pending_credits(&mut wallet_core).await?;
            let started = Instant::now();
            let (tx, _) = wallet_core
                .send_privacy_preserving_tx(accounts, 0, Program::serialize_message(message)?, &programs)
                .await
                .map_err(|e| anyhow::anyhow!("Update failed: {e:?}"))?;
            let submitted = started.elapsed().as_secs_f64();
            let block = fees::wait_included(&client, tx).await?;
            println!("TIME step=lc-update prove_and_submit_s={submitted:.1} included_s={:.1}", started.elapsed().as_secs_f64());
            follow_private_tx(&mut wallet_core, block).await?;
            report_refund("lc-update-refund", &credits, &pending_credits(&mut wallet_core).await?);
            fees::report_status(&client, "lc-update", tx, block).await?;
            report_published(&wallet_core, &client, "lc-update", (program, instance, record_account(&program, &instance, next_slot)), tx, vec![("payer".to_owned(), payer.as_ref().to_vec())]).await?;
            let held = read_instance(&client, program, instance).await?;
            println!("advanced in block {block} to slot {} block {}", held.store.finalized.slot, held.latest.block_number);
        }

        "attest-public" => {
            let (program, instance, getproof, payer) = (id(arg(1)?)?, id(arg(2)?)?, arg(3)?, id(arg(4)?)?);
            let case = a.get(5).map_or("honest", String::as_str);
            let query = storage_query(getproof, case)?;
            // The dry run: the check `Attest` makes, run here first against the record the
            // instance would use. A refused public attestation is charged its whole gas limit,
            // so an honest client stops on a refusal. The negative cases are sent anyway, to
            // test what the sequencer does with them.
            let slot = read_instance(&client, program, instance).await?.latest.slot;
            let held = read_record(&client, program, instance, slot).await?.ok_or_else(|| anyhow::anyhow!("no record at slot {slot}"))?;
            let ancestry = ancestry(getproof, &held, case)?;
            let verdict = match &ancestry {
                None => Ok(held.record.clone()),
                Some(a) => target_record(&held.record, a),
            }
            .and_then(|target| statement::evaluate(&target, &query))
            .map_or_else(|e| format!("{e:?}"), |_| "holds".to_owned());
            println!("DRYRUN step=lc-attest-public case={case} verdict={verdict}");
            if verdict != "holds" && !is_negative(case) {
                println!("ATTEST step=lc-attest-public case={case} outcome=refused-locally error={verdict}");
                return Ok(());
            }
            // `Attest` goes to the record account itself: one read, whatever the number of records.
            let message = LcMsg::Attest { instance, query, ancestry, reply: false };
            let account = record_mention(program, instance, slot);
            let before = fees::snapshot(&client, payer).await?;
            // Nothing is proven locally: the sequencer runs the check. So the time is submission
            // to inclusion, which the devnet's block interval mostly sets.
            let started = Instant::now();
            let tx = match wallet_core
                .send_pub_tx_paid_by(vec![account], 0, Program::serialize_message(message)?, Some(payer))
                .await
            {
                Ok(tx) => tx,
                Err(e) => {
                    println!("ATTEST step=lc-attest-public case={case} outcome=rejected error={e:?}");
                    return Ok(());
                }
            };
            let submitted = started.elapsed().as_secs_f64();
            let block = fees::wait_included(&client, tx).await?;
            println!("TIME step=lc-attest-public case={case} submit_s={submitted:.1} included_s={:.1}", started.elapsed().as_secs_f64());
            let step = if case == "honest" { "lc-attest-public".to_owned() } else { format!("lc-attest-public-{case}") };
            fees::report_public_tx(&client, &step, tx, before, fees::snapshot(&client, payer).await?).await?;
            report_attestation(&client, "lc-attest-public", case, tx, block).await?;
        }

        "attest-private" => {
            let bytecode = std::fs::read(arg(1)?)?;
            let (program, instance, getproof) = (id(arg(2)?)?, id(arg(3)?)?, arg(4)?);
            let (operator, payer) = (id(arg(5)?)?, id(arg(6)?)?);
            let case = a.get(7).map_or("honest", String::as_str);
            let held = read_instance(&client, program, instance).await?;
            let anchor = read_record(&client, program, instance, held.latest.slot)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no record at slot {}", held.latest.slot))?;
            let ancestry = ancestry(getproof, &anchor, case)?;
            let mut record = held.latest;
            if case == "unheld-record" {
                // The state root is the real one, so the proof checks out against it; only the
                // record account can tell this header isn't one the instance verified.
                record.slot += 1;
            }
            println!("attesting privately against block {} (slot {}), case {case}", record.block_number, record.slot);
            let query = storage_query(getproof, case)?;
            let mut secrets = query_secrets(&query);
            secrets.extend(ancestry_secrets(&ancestry));
            secrets.push(("payer".to_owned(), payer.as_ref().to_vec()));
            let slot = record.slot;
            let message = LcMsg::AttestPrivate { instance: Actor::new(instance, program), record, query, ancestry };
            let programs = ProgramCatalog::from([(program, Program::new(bytecode.into())?)]);
            let record_id = record_account(&program, &instance, slot);
            let accounts = vec![
                AccountIdentity::PrivateOwned(operator).select_program_actor_state(program),
                record_mention(program, instance, slot),
            ];
            let credits = pending_credits(&mut wallet_core).await?;
            let started = Instant::now();
            let tx = match wallet_core
                .send_privacy_preserving_tx(accounts, 0, Program::serialize_message(message)?, &programs)
                .await
            {
                Ok((tx, _)) => tx,
                Err(e) => {
                    println!(
                        "ATTEST step=lc-attest-private case={case} outcome=unprovable after_s={:.1} error={e:?}",
                        started.elapsed().as_secs_f64()
                    );
                    return Ok(());
                }
            };
            let submitted = started.elapsed().as_secs_f64();
            let Ok(block) = fees::wait_included_within(&client, tx, if is_negative(case) { 60 } else { 300 }).await else {
                println!("ATTEST step=lc-attest-private case={case} outcome=rejected after_s={submitted:.1}");
                return Ok(());
            };
            println!("TIME step=lc-attest-private case={case} prove_and_submit_s={submitted:.1} included_s={:.1}", started.elapsed().as_secs_f64());
            follow_private_tx(&mut wallet_core, block).await?;
            let step = if case == "honest" { "lc-attest-private-refund".to_owned() } else { format!("lc-attest-private-{case}-refund") };
            report_refund(&step, &credits, &pending_credits(&mut wallet_core).await?);
            report_attestation(&client, "lc-attest-private", case, tx, block).await?;
            report_published(&wallet_core, &client, "lc-attest-private", (program, instance, record_id), tx, secrets).await?;
        }

        // Which update in `chain-dir` would advance the instance as it stands on the node,
        // verified natively. Nothing is sent.
        "pending" => {
            let held = read_instance(&client, id(arg(1)?)?, id(arg(2)?)?).await?;
            match next_update(&held, &abs(arg(3)?)?) {
                Ok(n) => println!("next update verifies: slot {} -> {}, handoff {}", held.store.finalized.slot, n.store.finalized.slot, n.handoff),
                Err(e) => println!("no update yet: {e}"),
            }
        }

        "show" => {
            let (program, instance) = (id(arg(1)?)?, id(arg(2)?)?);
            let held = read_instance(&client, program, instance).await?;
            let written = read_record(&client, program, instance, held.latest.slot).await?.is_some_and(|r| r.record == held.latest);
            println!(
                "finalized slot {} block {}; period {}; next committee known {}; latest record at {} (written {written}), latest block {}",
                held.store.finalized.slot,
                held.latest.block_number,
                core::period(held.store.finalized.slot),
                held.store.next_committee.is_some(),
                record_account(&program, &instance, held.latest.slot),
                held.latest.block_number,
            );
        }

        // The publication check, on every private transaction already in a block.
        "published" => {
            let (program, instance, payer) = (id(arg(1)?)?, id(arg(2)?)?, id(arg(4)?)?);
            let number: u64 = arg(3)?.parse()?;
            let block = sequencer_service_rpc::RpcClient::get_block(&client, number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("block {number} not found"))?;
            for t in block.body.transactions.iter().filter(|t| matches!(t, LeeTransaction::PrivacyPreserving(_))) {
                let secrets = vec![("payer".to_owned(), payer.as_ref().to_vec())];
                let record = record_account(&program, &instance, read_instance(&client, program, instance).await?.latest.slot);
                report_published(&wallet_core, &client, &format!("block-{number}"), (program, instance, record), t.hash(), secrets).await?;
            }
        }

        // Everything Init and Update would verify, run natively, with nothing sent.
        "check" => {
            let dir = abs(arg(1)?)?;
            let chain = load_chain(&dir, CHAIN_ID)?;
            let bootstrap = oracle::bootstrap(&chain.bootstrap);
            let store = core::bootstrap(&chain.cfg, &chain.checkpoint, &bootstrap, &oracle::committee(chain.bootstrap.current_sync_committee()))
                .map_err(|e| anyhow::anyhow!("bootstrap refused: {e:?}"))?;
            let record = record_of(&chain.cfg, &bootstrap.header, None, 0).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            println!("bootstrap verifies: slot {} block {}", store.finalized.slot, record.block_number);
            match next_update_in(&chain, &store) {
                Ok(n) => println!("next update verifies: slot {} -> {}, handoff {}", store.finalized.slot, n.store.finalized.slot, n.handoff),
                Err(e) => println!("no update yet: {e}"),
            }
            return Ok(());
        }

        other => anyhow::bail!("unknown command {other:?}\n{USAGE}"),
    }
    Ok(())
}
