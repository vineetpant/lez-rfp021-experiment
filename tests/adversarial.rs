//! The RFP's adversarial cases (Verification Security #1 and #2), run natively through the core
//! the program runs, on recorded mainnet (`tests/data/mainnet-1904`). Each case asserts the core's
//! exact typed error, and that Helios's `consensus-core` refuses it too; each has an honest
//! control that both accept, so a refusal can't come from a broken fixture.
//!
//!   cargo test --release --test adversarial
//!
//! Updates are edited as Beacon API JSON, which Helios's types round-trip, and the roots a
//! fabricated chain needs are recomputed with Helios's own SSZ hashing.

mod support;

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G2Affine, G2Projective, Scalar};
use eth_lc_types::core;
use eth_lc_types::error::Refusal;
use eth_lc_types::oracle::{self, AnyUpdate, Committee};
use helios_consensus_core::types::{BeaconBlockHeader, ExecutionPayloadHeader, LightClientHeader};
use serde_json::{Value, json};
use sha2_09::{Digest, Sha256};
use support::{Step, boot, keys, mainnet, steps};
use tree_hash::TreeHash;

fn hex(b: &[u8]) -> String {
    format!("0x{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn h32(v: &Value) -> [u8; 32] {
    lez_rfp021_experiment::chain::hex_decode(v.as_str().unwrap()).unwrap().try_into().unwrap()
}

/// The update's JSON, its inner object, and back.
fn to_json(u: &AnyUpdate) -> Value {
    serde_json::to_value(u).unwrap()
}

fn from_json(v: Value) -> AnyUpdate {
    serde_json::from_value(v).unwrap()
}

fn inner(v: &mut Value) -> &mut Value {
    let key = v.as_object().unwrap().keys().next().unwrap().clone();
    &mut v[key]
}

/// The last recorded step: a finality update on top of the store two handoffs past the
/// checkpoint.
fn last_step() -> Step {
    steps(&mainnet()).0.pop().unwrap()
}

/// The core's verdict and Helios's on `update` in place of `step`'s.
fn both(step: &Step, update: &AnyUpdate) -> (Result<core::LightClientStore, Refusal>, bool) {
    let chain = mainnet();
    (step.apply_other(&chain, update).map(|(s, _)| s), step.helios(&chain, update).is_ok())
}

/// Refused by the core with `error`, and refused by Helios.
fn refused(step: &Step, update: &AnyUpdate, error: Refusal) {
    let (core, helios) = both(step, update);
    assert_eq!(core.map(|_| ()), Err(error));
    assert!(!helios, "Helios accepted what the core refused with {error:?}");
}

/// The root a Merkle branch proves `leaf` into: the inverse of the branch check.
fn branch_root(leaf: [u8; 32], branch: &[Value], depth: usize, index: u64) -> [u8; 32] {
    (0..depth).fold(leaf, |node, i| {
        let sib = h32(&branch[i]);
        let (l, r) = if (index >> i) & 1 == 1 { (sib, node) } else { (node, sib) };
        let mut h = Sha256::new();
        h.update(l);
        h.update(r);
        h.finalize().into()
    })
}

/// 512 keys we hold, signing as a sync committee.
struct Forger {
    sk: Scalar,
    committee: Value,
}

impl Forger {
    fn new() -> Self {
        let sks: Vec<Scalar> = (0..512u64).map(|i| Scalar::from(0xfa6e_0000_0000 + i * 104_729 + 3)).collect();
        let pks: Vec<String> = sks.iter().map(|s| hex(&G1Affine::from(G1Affine::generator() * s).to_compressed())).collect();
        let sk = sks.iter().fold(Scalar::zero(), |a, s| a + s);
        let aggregate = hex(&G1Affine::from(G1Affine::generator() * sk).to_compressed());
        Self { sk, committee: json!({ "pubkeys": pks, "aggregate_pubkey": aggregate }) }
    }

    fn committee(&self) -> Committee {
        serde_json::from_value(self.committee.clone()).unwrap()
    }

    fn keys(&self) -> Vec<u8> {
        oracle::committee(&self.committee())
    }

    fn root(&self) -> [u8; 32] {
        core::committee_root(&self.keys()).unwrap()
    }

    /// Signs `u`'s attested header, as all 512 members, under mainnet's Fulu domain.
    fn sign(&self, u: &mut Value) {
        let chain = mainnet();
        let attested: LightClientHeader = serde_json::from_value(u["attested_header"].clone()).unwrap();
        let fork_version = [6u8, 0, 0, 0];
        let mut fork_data = Sha256::new();
        fork_data.update([fork_version, [0; 4], [0; 4], [0; 4], [0; 4], [0; 4], [0; 4], [0; 4]].concat());
        fork_data.update(chain.config.chain.genesis_validators_root);
        let fork_data_root: [u8; 32] = fork_data.finalize().into();
        let mut domain = [0u8; 32];
        domain[0] = 7;
        domain[4..].copy_from_slice(&fork_data_root[..28]);
        let mut signing = Sha256::new();
        signing.update(attested.beacon().tree_hash_root().0);
        signing.update(domain);
        let msg: [u8; 32] = signing.finalize().into();
        let hm = <G2Projective as HashToCurve<ExpandMsgXmd<Sha256>>>::hash_to_curve(msg, b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_");
        u["sync_aggregate"]["sync_committee_bits"] = json!(hex(&[0xff; 64]));
        u["sync_aggregate"]["sync_committee_signature"] = json!(hex(&G2Affine::from(hm * self.sk).to_compressed()));
    }
}

/// A finality update for a block Ethereum never finalized, self-consistent throughout: a
/// fabricated execution state root, committed in the finalized header's body, that header in the
/// attested state, and the attested header signed by all 512 of `forger`'s keys.
fn fabricated(forger: &Forger) -> AnyUpdate {
    let mut v = to_json(&last_step().update);
    let u = inner(&mut v);
    u["finalized_header"]["execution"]["state_root"] = json!(hex(&[0xf0; 32]));
    let finalized: LightClientHeader = serde_json::from_value(u["finalized_header"].clone()).unwrap();
    let execution: &ExecutionPayloadHeader = finalized.execution().unwrap();
    let branch = u["finalized_header"]["execution_branch"].as_array().unwrap().clone();
    let body_root = branch_root(execution.tree_hash_root().0, &branch, 4, 9);
    u["finalized_header"]["beacon"]["body_root"] = json!(hex(&body_root));
    let beacon: BeaconBlockHeader = serde_json::from_value(u["finalized_header"]["beacon"].clone()).unwrap();
    let finality = u["finality_branch"].as_array().unwrap().clone();
    // Electra on: the finalized checkpoint root at depth 7, index 41.
    u["attested_header"]["beacon"]["state_root"] = json!(hex(&branch_root(beacon.tree_hash_root().0, &finality, 7, 41)));
    forger.sign(u);
    from_json(v)
}

// ---- controls ----

#[test]
fn honest_last_step_verifies_in_both() {
    let step = last_step();
    let (core, helios) = both(&step, &step.update);
    core.expect("the core accepts the real update");
    assert!(helios, "Helios accepts the real update");
}

#[test]
fn vs1_control_the_fabrication_is_self_consistent() {
    // Against a store that trusted the forger's committee, both accept the fabricated update, so
    // the refusals below come from the committee chain back to the checkpoint, not a broken
    // fixture.
    let forger = Forger::new();
    let mut step = last_step();
    step.store.current_sync_committee = forger.root();
    step.store.next_sync_committee = None;
    step.current = forger.keys();
    step.next = None;
    step.helios = oracle::helios_store(&step.helios.finalized_header, &forger.committee(), None);
    let (core, helios) = both(&step, &fabricated(&forger));
    core.expect("the core accepts it, against the forger's committee");
    assert!(helios, "Helios accepts it, against the forger's committee");
}

// ---- Verification Security #1: a fabricated chain ----

#[test]
fn vs1_fabricated_chain_is_rejected() {
    let forger = Forger::new();
    let mut step = last_step();
    let fake = fabricated(&forger);
    // With the real committee's keys, the forger's signature doesn't verify.
    refused(&step, &fake, Refusal::InvalidSignature);
    // The forger's keys don't hash to the committee the store holds.
    step.current = forger.keys();
    assert_eq!(step.apply_other(&mainnet(), &fake).map(|_| ()), Err(Refusal::CommitteeKeysMismatch));
}

#[test]
fn vs1_fabricated_checkpoint_is_rejected() {
    let chain = mainnet();
    let mut b = oracle::bootstrap(&chain.bootstrap);
    b.header.beacon.state_root = [0xf0; 32];
    let committee = oracle::committee(chain.bootstrap.current_sync_committee());
    assert_eq!(core::bootstrap(&chain.config, &b, &committee).map(|_| ()), Err(Refusal::CheckpointRootMismatch));
}

// ---- Verification Security #2 ----

#[test]
fn vs2_committee_never_handed_off_is_rejected() {
    // The last update is signed by a committee two handoffs past the checkpoint.
    let chain = mainnet();
    let (store, helios) = boot(&chain);
    let (current, next) = keys(&chain, &store);
    let update = last_step().update;
    let step = Step { store, helios, update: update.clone(), current, next };
    refused(&step, &update, Refusal::UnverifiedHandoff);
}

#[test]
fn vs2_forged_next_committee_is_rejected() {
    let step = steps(&mainnet()).0.into_iter().next().unwrap();
    let mut v = to_json(&step.update);
    inner(&mut v)["next_sync_committee"] = Forger::new().committee;
    refused(&step, &from_json(v), Refusal::InvalidCommitteeProof);
}

#[test]
fn vs2_below_participation_is_rejected() {
    let step = last_step();
    let mut v = to_json(&step.update);
    // 341 of 512 signers, one short of 2/3.
    let mut bits = [0u8; 64];
    for i in 0..341 {
        bits[i / 8] |= 1 << (i % 8);
    }
    inner(&mut v)["sync_aggregate"]["sync_committee_bits"] = json!(hex(&bits));
    refused(&step, &from_json(v), Refusal::InsufficientSigners);
}

#[test]
fn vs2_header_not_finalized_is_rejected() {
    let step = last_step();
    let mut v = to_json(&step.update);
    let mut leaf = h32(&inner(&mut v)["finality_branch"][0]);
    leaf[0] ^= 1;
    inner(&mut v)["finality_branch"][0] = json!(hex(&leaf));
    refused(&step, &from_json(v), Refusal::UnfinalizedHeader);
}

#[test]
fn vs2_tampered_execution_roots_are_rejected() {
    let step = last_step();
    for field in ["state_root", "receipts_root"] {
        let mut v = to_json(&step.update);
        inner(&mut v)["finalized_header"]["execution"][field] = json!(hex(&[0xf0; 32]));
        refused(&step, &from_json(v), Refusal::InvalidExecutionProof);
    }
}

#[test]
fn vs2_update_under_another_chain_is_rejected() {
    // A mainnet update replayed to an instance of another chain: its signature is over mainnet's
    // domain, so it doesn't verify under another genesis validators root.
    let step = last_step();
    let mut chain = mainnet();
    chain.config.chain.genesis_validators_root = [0xd8; 32];
    assert_eq!(step.apply_other(&chain, &step.update).map(|_| ()), Err(Refusal::InvalidSignature));
    assert!(step.helios(&chain, &step.update).is_err(), "Helios refuses it too");
}

// ---- also refused ----

#[test]
fn tampered_signature_and_replayed_update_are_rejected() {
    let step = last_step();
    let mut v = to_json(&step.update);
    let p = inner(&mut v)["attested_header"]["beacon"]["proposer_index"].as_str().unwrap().parse::<u64>().unwrap();
    inner(&mut v)["attested_header"]["beacon"]["proposer_index"] = json!((p ^ 1).to_string());
    refused(&step, &from_json(v), Refusal::InvalidSignature);
    // Applied once, then replayed on the store it produced.
    let chain = mainnet();
    let (after, _) = step.apply(&chain).unwrap();
    let helios_after = step.helios(&chain, &step.update).unwrap();
    let replay = Step { store: after, helios: helios_after, update: step.update.clone(), current: step.current.clone(), next: step.next.clone() };
    refused(&replay, &step.update, Refusal::UpdateNotNewer);
}

#[test]
fn committee_roots_equal_helios() {
    // The direct SSZ root the program uses, against Helios's own hashing, for every committee in
    // the segment and a generated one.
    let chain = mainnet();
    let mut all = chain.committees.clone();
    all.push(Forger::new().committee());
    for c in &all {
        assert_eq!(core::committee_root(&oracle::committee(c)).unwrap(), c.tree_hash_root().0);
    }
}

#[test]
fn min_participants_is_enforced() {
    let mut chain = mainnet();
    let b = oracle::bootstrap(&chain.bootstrap);
    let committee = oracle::committee(chain.bootstrap.current_sync_committee());
    // An instance may not ask for fewer than the specs' supermajority.
    chain.config.finality.min_participants = 341;
    assert_eq!(core::bootstrap(&chain.config, &b, &committee).map(|_| ()), Err(Refusal::MalformedInput));
    // It may ask for more: the finality update had 510 signers, so an instance requiring all
    // 512 refuses it, and one requiring 510 accepts it.
    let step = last_step();
    let signers = core::signers(&oracle::update(&step.update).sync_committee_bits);
    assert_eq!(signers, 510);
    chain.config.finality.min_participants = 512;
    assert_eq!(step.apply(&chain).map(|_| ()), Err(Refusal::InsufficientSigners));
    chain.config.finality.min_participants = 510;
    assert!(step.apply(&chain).is_ok());
}
