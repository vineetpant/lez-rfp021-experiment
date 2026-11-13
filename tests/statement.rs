//! Claims natively, against the finalised header the recorded mainnet segment ends on (block
//! 26376237): a WETH storage slot and its account, proven with `eth_getProof` at that block, and
//! receipt logs of that block. The statement IDs are checked against the properties proposal
//! section 4.7 gives them.
//!
//!   cargo test --release --test statement

mod support;

use alloy_primitives::U256;
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use eth_lc_types::FinalisedHeader;
use eth_lc_types::error::Refusal;
use eth_lc_types::statement::{Claim, Class, Condition, Fact, Op, Proof, Subject, check};
use lee_core::account::AccountId;
use lez_rfp021_experiment::chain::storage_proof;
use support::{encode_receipt, mainnet, receipt_proof, rlp, steps};

const INSTANCE: AccountId = AccountId::new([7; 32]);
const WHOLE: Subject = Subject::StorageValue { offset: 0, width: 32 };

/// The finalised header the last step writes.
fn header() -> FinalisedHeader {
    let chain = mainnet();
    let (mut s, _) = steps(&chain);
    let step = s.pop().unwrap();
    let u = eth_lc_types::oracle::update(&step.update);
    assert!(step.apply(&chain).unwrap().1, "the finality update finalises a new header");
    let signers = eth_lc_types::core::signers(&u.sync_committee_bits) as u16;
    eth_lc_types::finalised_header(&INSTANCE, &chain.config, &u.finalized_header, None, signers, eth_lc_types::core::sync_committee_period(u.signature_slot)).unwrap()
}

fn weth() -> Proof {
    storage_proof(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904/getproof-26376237.json")).unwrap()
}

fn weth_account() -> Proof {
    let Proof::Storage { address, account, account_proof, .. } = weth() else { unreachable!() };
    Proof::Account { address, account, account_proof }
}

fn word(n: u128) -> [u8; 32] {
    U256::from(n).to_be_bytes()
}

fn one(proof: Proof, conditions: Vec<Condition>) -> Claim {
    Claim { chain_id: 1, facts: vec![Fact { proof, conditions }] }
}

/// The WETH slot compared with zero.
fn slot(chain_id: u64, op: Op) -> Claim {
    Claim { chain_id, facts: vec![Fact { proof: weth(), conditions: vec![Condition { subject: WHOLE, op, rhs: [0; 32] }] }] }
}

#[test]
fn honest_storage_claim_verifies_and_is_deterministic() {
    let h = header();
    assert_eq!(h.block_number, 26_376_237);
    let s = check(&INSTANCE, &h, &slot(1, Op::Greater)).expect("the WETH slot is nonzero");
    assert_eq!(s, check(&INSTANCE, &h, &slot(1, Op::Greater)).unwrap(), "R.1: same inputs, same verdict and ID");
    assert_eq!(s.instance, INSTANCE, "F.15: the statement names its instance");
    assert_eq!(s.facts.len(), 1);
    assert_eq!(s.facts[0].class, Class::State);
    assert_eq!(s.facts[0].conditions, slot(1, Op::Greater).facts[0].conditions, "the conditions asserted travel with the fact");
}

#[test]
fn a_false_claim_is_refused() {
    assert_eq!(check(&INSTANCE, &header(), &slot(1, Op::Equal)), Err(Refusal::ConditionUnsatisfied));
}

#[test]
fn vs2_replay_against_another_chain_is_refused() {
    assert_eq!(check(&INSTANCE, &header(), &slot(11_155_111, Op::Greater)), Err(Refusal::WrongChain));
}

#[test]
fn vs2_condition_over_uncovered_state_is_refused() {
    // A value the proof doesn't establish: a log topic, or the account's balance, asked of a
    // storage proof; a packed field that runs off the slot.
    for subject in [Subject::LogTopic(0), Subject::AccountBalance, Subject::StorageValue { offset: 30, width: 4 }, Subject::StorageValue { offset: 0, width: 0 }] {
        let c = one(weth(), vec![Condition { subject, op: Op::Greater, rhs: [0; 32] }]);
        assert_eq!(check(&INSTANCE, &header(), &c), Err(Refusal::MalformedInput), "{subject:?}");
    }
    // No fact at all.
    assert_eq!(check(&INSTANCE, &header(), &Claim { chain_id: 1, facts: vec![] }), Err(Refusal::MalformedInput));
    // A slot the proof doesn't cover: the proof's nodes, claimed for another slot.
    let mut p = weth();
    let Proof::Storage { slot, .. } = &mut p else { unreachable!() };
    slot[31] ^= 1;
    assert_eq!(check(&INSTANCE, &header(), &one(p, vec![])), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn a_tampered_proof_or_root_is_refused() {
    let mut p = weth();
    let Proof::Storage { storage_proof, .. } = &mut p else { unreachable!() };
    *storage_proof.last_mut().unwrap().last_mut().unwrap() ^= 1;
    assert_eq!(check(&INSTANCE, &header(), &one(p, vec![])), Err(Refusal::InvalidInclusionProof));
    // A caller's root is never used: the header's is, and a wrong one fails.
    let mut h = header();
    h.state_root[0] ^= 1;
    assert_eq!(check(&INSTANCE, &h, &slot(1, Op::Greater)), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn packed_storage_fields() {
    let Proof::Storage { value, .. } = weth() else { unreachable!() };
    let h = header();
    // The low 16 bytes of the slot, as a uint128 packed at offset 0, and the high 16 bytes.
    let low = U256::from_be_slice(&value[16..]).to_be_bytes::<32>();
    let high = U256::from_be_slice(&value[..16]).to_be_bytes::<32>();
    let field = |offset, width, rhs| one(weth(), vec![Condition { subject: Subject::StorageValue { offset, width }, op: Op::Equal, rhs }]);
    check(&INSTANCE, &h, &field(0, 16, low)).expect("the low field");
    check(&INSTANCE, &h, &field(16, 16, high)).expect("the high field");
    check(&INSTANCE, &h, &field(0, 32, value)).expect("the whole slot");
}

#[test]
fn account_conditions_need_an_account_proof() {
    let h = header();
    let Proof::Storage { account, .. } = weth() else { unreachable!() };
    let c = one(
        weth_account(),
        vec![
            Condition { subject: Subject::AccountBalance, op: Op::Greater, rhs: [0; 32] },
            Condition { subject: Subject::AccountNonce, op: Op::AtLeast, rhs: word(1) },
            Condition { subject: Subject::AccountCodeHash, op: Op::Equal, rhs: account.code_hash },
        ],
    );
    let s = check(&INSTANCE, &h, &c).expect("WETH holds ether, has a nonce and its code");
    assert_eq!(s.facts[0].class, Class::State);
    let wrong = one(weth_account(), vec![Condition { subject: Subject::AccountCodeHash, op: Op::Equal, rhs: [0; 32] }]);
    assert_eq!(check(&INSTANCE, &h, &wrong), Err(Refusal::ConditionUnsatisfied));
    let mut tampered = weth_account();
    let Proof::Account { account, .. } = &mut tampered else { unreachable!() };
    account.balance[31] ^= 1;
    assert_eq!(check(&INSTANCE, &h, &one(tampered, vec![])), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn a_compound_claim_gives_one_id_per_fact() {
    let h = header();
    let c = Claim {
        chain_id: 1,
        facts: vec![
            Fact { proof: weth(), conditions: vec![Condition { subject: WHOLE, op: Op::Greater, rhs: [0; 32] }] },
            Fact { proof: weth_account(), conditions: vec![Condition { subject: Subject::AccountBalance, op: Op::Greater, rhs: [0; 32] }] },
        ],
    };
    let s = check(&INSTANCE, &h, &c).expect("both facts hold");
    assert_eq!(s.facts.len(), 2);
    assert_ne!(s.facts[0].id, s.facts[1].id, "a storage fact and an account fact are distinct kinds");
    // One fact failing fails the claim.
    let mut broken = c.clone();
    broken.facts[1].conditions[0].op = Op::Equal;
    assert_eq!(check(&INSTANCE, &h, &broken), Err(Refusal::ConditionUnsatisfied));
}

// ---- statement IDs (F.6) ----

#[test]
fn ids_depend_on_the_state_alone() {
    let h = header();
    let base = check(&INSTANCE, &h, &slot(1, Op::Greater)).unwrap().facts[0].id;
    // Other conditions over the same state, another instance, a later header of the same state.
    let other_conditions = check(&INSTANCE, &h, &slot(1, Op::NotEqual)).unwrap().facts[0].id;
    let other_instance = check(&AccountId::new([8; 32]), &h, &slot(1, Op::Greater)).unwrap().facts[0].id;
    let mut later = h.clone();
    later.slot += 32;
    later.block_number += 1;
    later.block_hash[0] ^= 1;
    let other_block = check(&INSTANCE, &later, &slot(1, Op::Greater)).unwrap().facts[0].id;
    assert_eq!(base, other_conditions, "the conditions don't enter the ID");
    assert_eq!(base, other_instance, "the instance doesn't enter the ID");
    assert_eq!(base, other_block, "identical state at another block: the same ID");
}

// ---- logs, by their position in a transaction receipt ----

/// A trie holding one receipt at index 0: its root, and the receipt's proof.
fn single_receipt(receipt: Vec<u8>) -> ([u8; 32], Proof) {
    let key = Nibbles::unpack(rlp(&0u64));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![key]));
    hb.add_leaf(key, &receipt);
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&key).into_iter().map(|(_, n)| n.to_vec()).collect();
    (root.0, Proof::Log { tx_index: 0, receipt, proof, log_index: 0 })
}

/// A receipt with one log, successful or reverted.
fn receipt_with(status: &str, topics: &[&str], data: &str) -> Vec<u8> {
    let bloom = format!("0x{}", "00".repeat(256));
    let log = serde_json::json!({ "address": format!("0x{}", "11".repeat(20)), "topics": topics, "data": data });
    encode_receipt(&serde_json::json!({ "type": "0x2", "status": status, "cumulativeGasUsed": "0x5208", "logsBloom": bloom, "logs": [log] }))
}

#[test]
fn honest_log_claim_verifies() {
    let h = header();
    let (root, proof) = receipt_proof(2);
    assert_eq!(root, h.receipts_root, "the rebuilt receipt trie matches the verified header");
    let log = check(&INSTANCE, &h, &one(proof, vec![])).expect("a real receipt log verifies");
    assert_eq!(log.facts[0].class, Class::Occurrence);
    let storage = check(&INSTANCE, &h, &slot(1, Op::Greater)).unwrap();
    assert_ne!(log.facts[0].id, storage.facts[0].id, "F.6: distinct state, distinct ID");
}

#[test]
fn a_log_proof_for_another_transaction_is_refused() {
    let (_, mut proof) = receipt_proof(2);
    let Proof::Log { tx_index, .. } = &mut proof else { unreachable!() };
    *tx_index = 3;
    assert_eq!(check(&INSTANCE, &header(), &one(proof, vec![])), Err(Refusal::InvalidInclusionProof));
}

#[test]
fn a_10_eth_deposit_and_its_id() {
    let event = format!("0x{}", "aa".repeat(32));
    let sender = format!("0x{}", "bb".repeat(32));
    let ten = 10u128 * 10u128.pow(18);
    let data = format!("0x{:064x}{:064x}", ten, 42);
    let (root, proof) = single_receipt(receipt_with("0x1", &[&event, &sender], &data));
    let mut h = header();
    h.receipts_root = root;
    let mut emitter = [0u8; 32];
    emitter[12..].copy_from_slice(&[0x11; 20]);
    let deposit = |amount: u128| {
        one(
            proof.clone(),
            vec![
                Condition { subject: Subject::LogEmitter, op: Op::Equal, rhs: emitter },
                Condition { subject: Subject::LogTopic(0), op: Op::Equal, rhs: [0xaa; 32] },
                Condition { subject: Subject::LogDataWord(0), op: Op::Equal, rhs: word(amount) },
                Condition { subject: Subject::LogTopicCount, op: Op::Equal, rhs: word(2) },
                Condition { subject: Subject::LogDataLength, op: Op::Equal, rhs: word(64) },
            ],
        )
    };
    let s = check(&INSTANCE, &h, &deposit(ten)).expect("a 10 ETH deposit");
    assert_eq!(check(&INSTANCE, &h, &deposit(ten + 1)), Err(Refusal::ConditionUnsatisfied));
    // Data word 2 doesn't exist: malformed, not zero.
    let beyond = one(proof.clone(), vec![Condition { subject: Subject::LogDataWord(2), op: Op::Equal, rhs: [0; 32] }]);
    assert_eq!(check(&INSTANCE, &h, &beyond), Err(Refusal::MalformedInput));
    // The same log at another block is another occurrence.
    let mut other_block = h.clone();
    other_block.block_hash[0] ^= 1;
    assert_ne!(s.facts[0].id, check(&INSTANCE, &other_block, &deposit(ten)).unwrap().facts[0].id);
    // Moving the topic/data boundary keeps the bytes but changes the log: a different ID.
    let (moved_root, moved) = single_receipt(receipt_with("0x1", &[&event], &format!("{}{}", sender, &data[2..])));
    let mut m = h.clone();
    m.receipts_root = moved_root;
    assert_ne!(s.facts[0].id, check(&INSTANCE, &m, &one(moved, vec![])).unwrap().facts[0].id, "length-prefixed: no boundary collision");
}

#[test]
fn a_reverted_transactions_log_proves_nothing() {
    let (root, proof) = single_receipt(receipt_with("0x0", &[], "0x"));
    let mut h = header();
    h.receipts_root = root;
    assert_eq!(check(&INSTANCE, &h, &one(proof, vec![])), Err(Refusal::ConditionUnsatisfied));
}
