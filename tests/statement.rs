//! Attestations natively, against the record the recorded mainnet segment ends on (block
//! 26376237): a WETH storage slot, proven with `eth_getProof` at that block.
//!
//!   cargo test --release --test statement

mod support;

use eth_lc_types::HeaderRecord;
use eth_lc_types::error::Error;
use eth_lc_types::statement::{Cmp, Field, Predicate, Proof, Query, evaluate};
use lez_hello_world::chain::storage_proof;
use support::{mainnet, steps};

/// The record the last step writes.
fn record() -> HeaderRecord {
    let chain = mainnet();
    let (mut s, _) = steps(&chain);
    let (_, record) = s.pop().unwrap().apply(&chain).unwrap();
    record.expect("the finality update finalizes a new header")
}

fn query(chain_id: u64, field: Field, cmp: Cmp) -> Query {
    let proof = storage_proof(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904/getproof-26376237.json")).unwrap();
    Query { chain_id, proof: Proof::Storage(proof), predicates: vec![Predicate { field, cmp, value: [0; 32] }] }
}

#[test]
fn honest_storage_attestation_verifies_and_is_deterministic() {
    let r = record();
    assert_eq!(r.block_number, 26_376_237);
    let a = evaluate(&r, &query(1, Field::SlotValue, Cmp::Gt)).expect("the WETH slot is nonzero");
    assert_eq!(a, evaluate(&r, &query(1, Field::SlotValue, Cmp::Gt)).unwrap(), "R.1: same inputs, same verdict and ID");
}

#[test]
fn a_false_claim_is_refused() {
    assert_eq!(evaluate(&record(), &query(1, Field::SlotValue, Cmp::Eq)), Err(Error::PredicateFalse));
}

#[test]
fn vs2_replay_against_another_chain_is_refused() {
    assert_eq!(evaluate(&record(), &query(11_155_111, Field::SlotValue, Cmp::Gt)), Err(Error::ChainMismatch));
}

#[test]
fn vs2_predicate_over_uncovered_state_is_refused() {
    // A field the proof doesn't carry: a log topic, asked of a storage proof.
    assert_eq!(evaluate(&record(), &query(1, Field::LogTopic(0), Cmp::Gt)), Err(Error::Malformed));
    // A slot the proof doesn't cover: the proof's nodes, claimed for another slot.
    let mut q = query(1, Field::SlotValue, Cmp::Gt);
    let Proof::Storage(p) = &mut q.proof else { unreachable!() };
    p.slot[31] ^= 1;
    assert_eq!(evaluate(&record(), &q), Err(Error::BadInclusion));
}

#[test]
fn a_tampered_proof_or_root_is_refused() {
    let mut q = query(1, Field::SlotValue, Cmp::Gt);
    let Proof::Storage(p) = &mut q.proof else { unreachable!() };
    let leaf = p.storage_proof.last_mut().unwrap();
    *leaf.last_mut().unwrap() ^= 1;
    assert_eq!(evaluate(&record(), &q), Err(Error::BadInclusion));
    // A caller's root is never used: the record's is, and a wrong one fails.
    let mut r = record();
    r.state_root[0] ^= 1;
    assert_eq!(evaluate(&r, &query(1, Field::SlotValue, Cmp::Gt)), Err(Error::BadInclusion));
}
