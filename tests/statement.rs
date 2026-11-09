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

// ---- a log, by its position in a transaction receipt ----

use alloy_rlp::{BufMut, Encodable, Header};
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use eth_lc_types::mpt::ReceiptProof;
use lez_hello_world::chain::hex_decode;
use serde_json::Value;

fn hex(v: &Value) -> Vec<u8> {
    hex_decode(v.as_str().unwrap()).unwrap()
}

fn uint(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.concat();
    let mut out = Vec::new();
    Header { list: true, payload_length: payload.len() }.encode(&mut out);
    out.put_slice(&payload);
    out
}

fn rlp<T: Encodable + ?Sized>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.encode(&mut out);
    out
}

/// A receipt from `eth_getBlockReceipts`, encoded as it sits in the receipt trie (EIP-2718).
fn encode_receipt(r: &Value) -> Vec<u8> {
    let logs: Vec<Vec<u8>> = r["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            let topics: Vec<Vec<u8>> = l["topics"].as_array().unwrap().iter().map(|t| rlp(&hex(t)[..])).collect();
            list(&[rlp(&hex(&l["address"])[..]), list(&topics), rlp(&hex(&l["data"])[..])])
        })
        .collect();
    let body = list(&[rlp(&uint(&r["status"])), rlp(&uint(&r["cumulativeGasUsed"])), rlp(&hex(&r["logsBloom"])[..]), list(&logs)]);
    match uint(&r["type"]) {
        0 => body,
        t => [vec![t as u8], body].concat(),
    }
}

/// A proof of transaction `tx`'s receipt, built from the block's receipts: the trie root it
/// rebuilds, and the proof.
fn receipt_proof(tx: u64) -> ([u8; 32], ReceiptProof) {
    let j: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904/receipts-26376237.json")).unwrap()).unwrap();
    let receipts = j["result"].as_array().unwrap();
    let mut leaves: Vec<(Nibbles, Vec<u8>)> = receipts.iter().enumerate().map(|(i, r)| (Nibbles::unpack(rlp(&(i as u64))), encode_receipt(r))).collect();
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let target = Nibbles::unpack(rlp(&tx));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![target]));
    for (k, v) in &leaves {
        hb.add_leaf(*k, v);
    }
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&target).into_iter().map(|(_, n)| n.to_vec()).collect();
    (root.0, ReceiptProof { tx_index: tx, receipt: encode_receipt(&receipts[tx as usize]), proof, log_index: 0 })
}

#[test]
fn honest_log_attestation_verifies() {
    let r = record();
    let (root, proof) = receipt_proof(2);
    assert_eq!(root, r.receipts_root, "the rebuilt receipt trie matches the verified record");
    let q = Query { chain_id: 1, proof: Proof::Log(proof), predicates: vec![Predicate { field: Field::LogAddress, cmp: Cmp::Ne, value: [0; 32] }] };
    let log = evaluate(&r, &q).expect("a real receipt log verifies");
    let storage = evaluate(&r, &query(1, Field::SlotValue, Cmp::Gt)).unwrap();
    assert_ne!(log.id, storage.id, "F.6: distinct state, distinct ID");
}

#[test]
fn a_log_proof_for_another_transaction_is_refused() {
    let (_, mut proof) = receipt_proof(2);
    proof.tx_index = 3;
    let q = Query { chain_id: 1, proof: Proof::Log(proof), predicates: vec![] };
    assert_eq!(evaluate(&record(), &q), Err(Error::BadInclusion));
}
