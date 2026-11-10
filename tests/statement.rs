//! Attestations natively, against the record the recorded mainnet segment ends on (block
//! 26376237): a WETH storage slot and its account, proven with `eth_getProof` at that block, and
//! receipt logs of that block. The statement IDs are checked against the properties
//! `rfp-021/statements.md` gives them.
//!
//!   cargo test --release --test statement

mod support;

use eth_lc_types::HeaderRecord;
use eth_lc_types::error::Refusal;
use eth_lc_types::mpt::{AccountProof, StorageProof};
use eth_lc_types::statement::{Attestation, Class, Op, Predicate, Proof, check};
use lee_core::account::AccountId;
use lez_hello_world::chain::storage_proof;
use support::{mainnet, steps};

const INSTANCE: AccountId = AccountId::new([7; 32]);

/// The record the last step writes.
fn record() -> HeaderRecord {
    let chain = mainnet();
    let (mut s, _) = steps(&chain);
    let step = s.pop().unwrap();
    let u = eth_lc_types::oracle::update(&step.update);
    assert!(step.apply(&chain).unwrap().1, "the finality update finalizes a new header");
    eth_lc_types::record_of(&chain.cfg, &u.finalized_header, None, eth_lc_types::core::signers(&u.sync_committee_bits) as u16).unwrap()
}

fn weth() -> StorageProof {
    storage_proof(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904/getproof-26376237.json")).unwrap()
}

fn weth_account() -> AccountProof {
    let p = weth();
    AccountProof { address: p.address, account: p.account, account_proof: p.account_proof }
}

/// The WETH slot compared with zero.
fn slot(chain_id: u64, op: Op) -> Attestation {
    Attestation { chain_id, proofs: vec![Proof::Storage(weth())], predicates: vec![Predicate::CheckSlotValue { proof: 0, op, value: [0; 32] }] }
}

#[test]
fn honest_storage_attestation_verifies_and_is_deterministic() {
    let r = record();
    assert_eq!(r.block_number, 26_376_237);
    let a = check(&INSTANCE, &r, &slot(1, Op::Greater)).expect("the WETH slot is nonzero");
    assert_eq!(a, check(&INSTANCE, &r, &slot(1, Op::Greater)).unwrap(), "R.1: same inputs, same verdict and ID");
    assert_eq!(a.instance, INSTANCE, "F.15: the statement names its instance");
    assert_eq!(a.facts.len(), 1);
    assert_eq!(a.facts[0].class, Class::State);
}

#[test]
fn a_false_claim_is_refused() {
    assert_eq!(check(&INSTANCE, &record(), &slot(1, Op::Equal)), Err(Refusal::ConditionUnsatisfied));
}

#[test]
fn vs2_replay_against_another_chain_is_refused() {
    assert_eq!(check(&INSTANCE, &record(), &slot(11_155_111, Op::Greater)), Err(Refusal::WrongChain));
}

#[test]
fn vs2_predicate_over_uncovered_state_is_refused() {
    // A value the proof doesn't establish: a log topic, or the account's balance, asked of a
    // storage proof; and a predicate naming a proof the attestation doesn't have.
    for predicate in [
        Predicate::CheckLogTopic { proof: 0, index: 0, op: Op::Greater, value: [0; 32] },
        Predicate::CheckAccountBalance { proof: 0, op: Op::Greater, value: [0; 32] },
        Predicate::CheckSlotValue { proof: 1, op: Op::Greater, value: [0; 32] },
    ] {
        let a = Attestation { chain_id: 1, proofs: vec![Proof::Storage(weth())], predicates: vec![predicate] };
        assert_eq!(check(&INSTANCE, &record(), &a), Err(Refusal::MalformedInput));
    }
    // No proof at all.
    let a = Attestation { chain_id: 1, proofs: vec![], predicates: vec![] };
    assert_eq!(check(&INSTANCE, &record(), &a), Err(Refusal::MalformedInput));
    // A slot the proof doesn't cover: the proof's nodes, claimed for another slot.
    let mut p = weth();
    p.slot[31] ^= 1;
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Storage(p)], predicates: vec![] };
    assert_eq!(check(&INSTANCE, &record(), &a), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn a_tampered_proof_or_root_is_refused() {
    let mut p = weth();
    let leaf = p.storage_proof.last_mut().unwrap();
    *leaf.last_mut().unwrap() ^= 1;
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Storage(p)], predicates: vec![] };
    assert_eq!(check(&INSTANCE, &record(), &a), Err(Refusal::InvalidInclusionProof));
    // A caller's root is never used: the record's is, and a wrong one fails.
    let mut r = record();
    r.state_root[0] ^= 1;
    assert_eq!(check(&INSTANCE, &r, &slot(1, Op::Greater)), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn account_predicates_need_an_account_proof() {
    let r = record();
    let a = Attestation {
        chain_id: 1,
        proofs: vec![Proof::Account(weth_account())],
        predicates: vec![
            Predicate::CheckAccountBalance { proof: 0, op: Op::Greater, value: [0; 32] },
            Predicate::CheckAccountNonce { proof: 0, op: Op::AtLeast, value: 1 },
            Predicate::CheckAccountCode { proof: 0, code_hash: weth().account.code_hash },
        ],
    };
    let s = check(&INSTANCE, &r, &a).expect("WETH holds ether, has a nonce and its code");
    assert_eq!(s.facts[0].class, Class::State);
    let mut wrong = a.clone();
    wrong.predicates = vec![Predicate::CheckAccountCode { proof: 0, code_hash: [0; 32] }];
    assert_eq!(check(&INSTANCE, &r, &wrong), Err(Refusal::ConditionUnsatisfied));
    let mut tampered = weth_account();
    tampered.account.balance[31] ^= 1;
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Account(tampered)], predicates: vec![] };
    assert_eq!(check(&INSTANCE, &r, &a), Err(Refusal::InvalidInclusionProof));
}

// ---- statement IDs (F.6) ----

#[test]
fn ids_depend_on_the_state_alone() {
    let r = record();
    let base = check(&INSTANCE, &r, &slot(1, Op::Greater)).unwrap().facts[0];
    // Another predicate over the same state, another instance, a later record of the same state.
    let other_predicate = check(&INSTANCE, &r, &slot(1, Op::NotEqual)).unwrap().facts[0];
    let other_instance = check(&AccountId::new([8; 32]), &r, &slot(1, Op::Greater)).unwrap().facts[0];
    let mut later = r.clone();
    later.slot += 32;
    later.block_number += 1;
    later.block_hash[0] ^= 1;
    let other_block = check(&INSTANCE, &later, &slot(1, Op::Greater)).unwrap().facts[0];
    assert_eq!(base, other_predicate, "the predicate doesn't enter the ID");
    assert_eq!(base, other_instance, "the instance doesn't enter the ID");
    assert_eq!(base, other_block, "identical state at another block: the same ID");
    // The same account as a storage fact and as an account fact: distinct kinds, distinct IDs.
    let account = Attestation { chain_id: 1, proofs: vec![Proof::Account(weth_account())], predicates: vec![] };
    assert_ne!(base.id, check(&INSTANCE, &r, &account).unwrap().facts[0].id);
}

// ---- a log, by its position in a transaction receipt ----

use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use eth_lc_types::mpt::ReceiptProof;
use support::{encode_receipt, receipt_proof, rlp};

/// A trie holding one receipt at index 0: its root, and the receipt's proof.
fn single_receipt(receipt: Vec<u8>) -> ([u8; 32], ReceiptProof) {
    let key = Nibbles::unpack(rlp(&0u64));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![key]));
    hb.add_leaf(key, &receipt);
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&key).into_iter().map(|(_, n)| n.to_vec()).collect();
    (root.0, ReceiptProof { tx_index: 0, receipt, proof, log_index: 0 })
}

/// A successful legacy receipt with one log.
fn receipt_with(topics: &[&str], data: &str) -> Vec<u8> {
    let bloom = format!("0x{}", "00".repeat(256));
    let log = serde_json::json!({ "address": format!("0x{}", "11".repeat(20)), "topics": topics, "data": data });
    encode_receipt(&serde_json::json!({ "type": "0x0", "status": "0x1", "cumulativeGasUsed": "0x5208", "logsBloom": bloom, "logs": [log] }))
}

#[test]
fn honest_log_attestation_verifies() {
    let r = record();
    let (root, proof) = receipt_proof(2);
    assert_eq!(root, r.receipts_root, "the rebuilt receipt trie matches the verified record");
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Receipt(proof)], predicates: vec![] };
    let log = check(&INSTANCE, &r, &a).expect("a real receipt log verifies");
    assert_eq!(log.facts[0].class, Class::Occurrence);
    let storage = check(&INSTANCE, &r, &slot(1, Op::Greater)).unwrap();
    assert_ne!(log.facts[0].id, storage.facts[0].id, "F.6: distinct state, distinct ID");
}

#[test]
fn a_log_proof_for_another_transaction_is_refused() {
    let (_, mut proof) = receipt_proof(2);
    proof.tx_index = 3;
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Receipt(proof)], predicates: vec![] };
    assert_eq!(check(&INSTANCE, &record(), &a), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn log_predicates_and_their_ids() {
    let t0 = format!("0x{}", "aa".repeat(32));
    let t1 = format!("0x{}", "bb".repeat(32));
    let amount = format!("0x{:064x}", 10u128 * 10u128.pow(18));
    let (root, proof) = single_receipt(receipt_with(&[&t0], &format!("{}{}", t1, &amount[2..])));
    let mut r = record();
    r.receipts_root = root;
    let deposit = |op: Op, value: u128| Attestation {
        chain_id: 1,
        proofs: vec![Proof::Receipt(proof.clone())],
        predicates: vec![
            Predicate::CheckLogAddress { proof: 0, address: [0x11; 20] },
            Predicate::CheckLogTopic { proof: 0, index: 0, op: Op::Equal, value: [0xaa; 32] },
            Predicate::CheckLogData { proof: 0, word: 1, op, value: alloy_primitives::U256::from(value).to_be_bytes() },
        ],
    };
    let ten = 10u128 * 10u128.pow(18);
    let s = check(&INSTANCE, &r, &deposit(Op::Equal, ten)).expect("a 10 ETH deposit");
    assert_eq!(check(&INSTANCE, &r, &deposit(Op::Equal, ten + 1)), Err(Refusal::ConditionUnsatisfied));
    // Data word 2 doesn't exist: malformed, not zero.
    let mut beyond = deposit(Op::Equal, ten);
    beyond.predicates[2] = Predicate::CheckLogData { proof: 0, word: 2, op: Op::Equal, value: [0; 32] };
    assert_eq!(check(&INSTANCE, &r, &beyond), Err(Refusal::MalformedInput));
    // The same log at another block is another occurrence.
    let mut other_block = r.clone();
    other_block.block_hash[0] ^= 1;
    assert_ne!(s.facts[0].id, check(&INSTANCE, &other_block, &deposit(Op::Equal, ten)).unwrap().facts[0].id);
    // Moving the topic/data boundary keeps the bytes but changes the log: a different ID.
    let (moved_root, moved) = single_receipt(receipt_with(&[&t0, &t1], &amount));
    let mut m = r.clone();
    m.receipts_root = moved_root;
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Receipt(moved)], predicates: vec![] };
    assert_ne!(s.facts[0].id, check(&INSTANCE, &m, &a).unwrap().facts[0].id, "length-prefixed: no boundary collision");
}

#[test]
fn a_reverted_transactions_log_proves_nothing() {
    let bloom = format!("0x{}", "00".repeat(256));
    let log = serde_json::json!({ "address": format!("0x{}", "11".repeat(20)), "topics": [], "data": "0x" });
    let receipt = encode_receipt(&serde_json::json!({ "type": "0x2", "status": "0x0", "cumulativeGasUsed": "0x5208", "logsBloom": bloom, "logs": [log] }));
    let (root, proof) = single_receipt(receipt);
    let mut r = record();
    r.receipts_root = root;
    let a = Attestation { chain_id: 1, proofs: vec![Proof::Receipt(proof)], predicates: vec![] };
    assert_eq!(check(&INSTANCE, &r, &a), Err(Refusal::ConditionUnsatisfied));
}
