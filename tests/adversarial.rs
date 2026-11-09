//! The RFP's adversarial cases (Verification Security #1 and #2), run natively through the
//! checks the program runs (`eth_lc_types::verify` over Helios `consensus-core`), on recorded
//! mainnet (`tests/data/mainnet-1904`). Each case asserts the exact typed error, and each has an
//! honest control that passes, so a refusal can't come from a broken fixture.
//!
//!   cargo test --release --test adversarial
//!
//! Updates are edited as Beacon API JSON, which Helios's types round-trip, and the roots a
//! fabricated chain needs are recomputed with Helios's own SSZ hashing.

mod support;

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G2Affine, G2Projective, Scalar};
use eth_lc_types::error::Error;
use eth_lc_types::verify::{self, AnyUpdate, cbor, committee_root};
use helios_consensus_core::types::{BeaconBlockHeader, ExecutionPayloadHeader, LightClientHeader};
use serde_json::{Value, json};
use sha2_09::{Digest, Sha256};
use support::{Step, keys, mainnet, steps};
use tree_hash::TreeHash;

fn hex(b: &[u8]) -> String {
    format!("0x{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn h32(v: &Value) -> [u8; 32] {
    lez_hello_world::chain::hex_decode(v.as_str().unwrap()).unwrap().try_into().unwrap()
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
fn last_step() -> (eth_lc_types::Store, AnyUpdate, Vec<u8>, Option<Vec<u8>>) {
    let chain = mainnet();
    let (mut s, _) = steps(&chain);
    let Step { store, update, current, next } = s.pop().unwrap();
    (store, update, current, next)
}

fn apply(update: &AnyUpdate, store: &eth_lc_types::Store, current: &[u8], next: Option<&[u8]>) -> Result<eth_lc_types::Store, Error> {
    let chain = mainnet();
    verify::update(&chain.cfg, store, &cbor(update), current, next, None).map(|(s, _)| s)
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

    fn keys(&self) -> Vec<u8> {
        cbor(&serde_json::from_value::<verify::Committee>(self.committee.clone()).unwrap())
    }

    fn root(&self) -> [u8; 32] {
        committee_root(&serde_json::from_value(self.committee.clone()).unwrap())
    }

    /// Signs `u`'s attested header, as all 512 members, under mainnet's Fulu domain.
    fn sign(&self, u: &mut Value) {
        let chain = mainnet();
        let attested: LightClientHeader = serde_json::from_value(u["attested_header"].clone()).unwrap();
        let fork_version = [6u8, 0, 0, 0];
        let mut fork_data = Sha256::new();
        fork_data.update([fork_version, [0; 4], [0; 4], [0; 4], [0; 4], [0; 4], [0; 4], [0; 4]].concat());
        fork_data.update(chain.cfg.genesis_validators_root);
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
    let (_, update, _, _) = last_step();
    let mut v = to_json(&update);
    let u = inner(&mut v);
    u["finalized_header"]["execution"]["state_root"] = json!(hex(&[0xf0; 32]));
    let finalized: LightClientHeader = serde_json::from_value(u["finalized_header"].clone()).unwrap();
    let execution: &ExecutionPayloadHeader = finalized.execution().unwrap();
    let branch = u["finalized_header"]["execution_branch"].as_array().unwrap().clone();
    let body_root = branch_root(execution.tree_hash_root().0, &branch, 4, 9);
    u["finalized_header"]["beacon"]["body_root"] = json!(hex(&body_root));
    let beacon: BeaconBlockHeader = serde_json::from_value(u["finalized_header"]["beacon"].clone()).unwrap();
    let finality = u["finality_branch"].as_array().unwrap().clone();
    // Electra on: the finalized checkpoint root at generalized index 105, depth 7, index 41.
    u["attested_header"]["beacon"]["state_root"] = json!(hex(&branch_root(beacon.tree_hash_root().0, &finality, 7, 41)));
    forger.sign(u);
    from_json(v)
}

// ---- controls ----

#[test]
fn honest_last_step_verifies() {
    let (store, update, current, next) = last_step();
    apply(&update, &store, &current, next.as_deref()).expect("the real update verifies");
}

#[test]
fn vs1_control_the_fabrication_is_self_consistent() {
    // Against a store that trusted the forger's committee, the fabricated update verifies, so the
    // refusals below come from the committee chain back to the checkpoint, not a broken fixture.
    let forger = Forger::new();
    let (mut store, _, _, _) = last_step();
    store.current_committee = forger.root();
    store.next_committee = None;
    let after = apply(&fabricated(&forger), &store, &forger.keys(), None).expect("self-consistent");
    assert!(after.finalized_slot > 0);
}

// ---- Verification Security #1: a fabricated chain ----

#[test]
fn vs1_fabricated_chain_is_rejected() {
    let forger = Forger::new();
    let (store, _, current, next) = last_step();
    let fake = fabricated(&forger);
    // The forger's keys don't hash to the committee the store holds.
    assert_eq!(apply(&fake, &store, &forger.keys(), next.as_deref()), Err(Error::CommitteeMismatch));
    // With the real committee's keys, the forger's signature doesn't verify.
    assert_eq!(apply(&fake, &store, &current, next.as_deref()), Err(Error::BadSignature));
}

#[test]
fn vs1_fabricated_checkpoint_is_rejected() {
    let chain = mainnet();
    let mut b = serde_json::to_value(&chain.bootstrap).unwrap();
    b["header"]["beacon"]["state_root"] = json!(hex(&[0xf0; 32]));
    let bootstrap: helios_consensus_core::types::Bootstrap<helios_consensus_core::consensus_spec::MainnetConsensusSpec> =
        serde_json::from_value(b).unwrap();
    assert_eq!(verify::bootstrap(&chain.cfg, &chain.checkpoint, &cbor(&bootstrap), None).map(|_| ()), Err(Error::CheckpointMismatch));
}

// ---- Verification Security #2 ----

#[test]
fn vs2_committee_never_handed_off_is_rejected() {
    // The last update is signed by a committee two handoffs past the checkpoint.
    let chain = mainnet();
    let (boot, _) = verify::bootstrap(&chain.cfg, &chain.checkpoint, &cbor(&chain.bootstrap), None).unwrap();
    let (_, update, _, _) = last_step();
    let (current, next) = keys(&chain, &boot);
    assert_eq!(apply(&update, &boot, &current, next.as_deref()), Err(Error::HandoffNotVerified));
}

#[test]
fn vs2_forged_next_committee_is_rejected() {
    let chain = mainnet();
    let (s, _) = steps(&chain);
    let Step { store, update, current, next } = s.into_iter().next().unwrap();
    let mut v = to_json(&update);
    inner(&mut v)["next_sync_committee"] = Forger::new().committee;
    assert_eq!(apply(&from_json(v), &store, &current, next.as_deref()), Err(Error::BadCommitteeBranch));
}

#[test]
fn vs2_below_participation_is_rejected() {
    let (store, update, current, next) = last_step();
    let mut v = to_json(&update);
    // 341 of 512 signers, one short of 2/3.
    let mut bits = [0u8; 64];
    for i in 0..341 {
        bits[i / 8] |= 1 << (i % 8);
    }
    inner(&mut v)["sync_aggregate"]["sync_committee_bits"] = json!(hex(&bits));
    assert_eq!(apply(&from_json(v), &store, &current, next.as_deref()), Err(Error::BelowParticipation));
}

#[test]
fn vs2_header_not_finalized_is_rejected() {
    let (store, update, current, next) = last_step();
    let mut v = to_json(&update);
    let mut leaf = h32(&inner(&mut v)["finality_branch"][0]);
    leaf[0] ^= 1;
    inner(&mut v)["finality_branch"][0] = json!(hex(&leaf));
    assert_eq!(apply(&from_json(v), &store, &current, next.as_deref()), Err(Error::NotFinalized));
}

#[test]
fn vs2_tampered_execution_roots_are_rejected() {
    let (store, update, current, next) = last_step();
    for field in ["state_root", "receipts_root"] {
        let mut v = to_json(&update);
        inner(&mut v)["finalized_header"]["execution"][field] = json!(hex(&[0xf0; 32]));
        assert_eq!(apply(&from_json(v), &store, &current, next.as_deref()), Err(Error::BadExecutionBranch), "{field}");
    }
}

#[test]
fn vs2_update_under_another_chain_is_rejected() {
    // A mainnet update replayed to an instance of another chain: its signature is over mainnet's
    // domain, so it doesn't verify under another genesis validators root.
    let (store, update, current, next) = last_step();
    let mut cfg = mainnet().cfg;
    cfg.genesis_validators_root = [0xd8; 32];
    assert_eq!(
        verify::update(&cfg, &store, &cbor(&update), &current, next.as_deref(), None).map(|_| ()),
        Err(Error::BadSignature)
    );
}

// ---- also refused ----

#[test]
fn tampered_signature_and_replayed_update_are_rejected() {
    let (store, update, current, next) = last_step();
    let mut v = to_json(&update);
    let p = inner(&mut v)["attested_header"]["beacon"]["proposer_index"].as_str().unwrap().parse::<u64>().unwrap();
    inner(&mut v)["attested_header"]["beacon"]["proposer_index"] = json!((p ^ 1).to_string());
    assert_eq!(apply(&from_json(v), &store, &current, next.as_deref()), Err(Error::BadSignature));
    let after = apply(&update, &store, &current, next.as_deref()).unwrap();
    assert_eq!(apply(&update, &after, &current, next.as_deref()), Err(Error::AlreadyApplied));
}
