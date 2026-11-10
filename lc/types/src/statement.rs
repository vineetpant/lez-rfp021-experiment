//! Attestations and the statements they produce (`rfp-021/statements.md`).
//!
//! A consumer program sends an [`Attestation`]: Merkle-Patricia proofs against one verified
//! header, and typed predicates over the values those proofs establish, all of which must hold
//! (F.3, F.4, soft #4). [`check`] verifies every proof, evaluates every predicate and returns a
//! [`Statement`] carrying one identifier per proof, derived from the proven Ethereum state alone,
//! for a consumer to key its nullifier on (F.5, F.6, F.15).

use crate::error::Refusal;
use crate::mpt::{self, AccountProof, Log, ReceiptProof, StorageProof};
use crate::{HeaderRecord, Root};
use alloy_primitives::U256;
use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::account::AccountId;
use sha2::{Digest, Sha256};

/// Domain of every statement ID; a new encoding gets a new version.
const ID_DOMAIN: &[u8] = b"eth_lc/statement/v1";

/// A comparison between a proven value and the consumer's, as unsigned 256-bit integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Op {
    Equal,
    NotEqual,
    Less,
    AtMost,
    Greater,
    AtLeast,
}

impl Op {
    fn compare(self, proven: U256, expected: U256) -> bool {
        match self {
            Op::Equal => proven == expected,
            Op::NotEqual => proven != expected,
            Op::Less => proven < expected,
            Op::AtMost => proven <= expected,
            Op::Greater => proven > expected,
            Op::AtLeast => proven >= expected,
        }
    }
}

/// One Merkle-Patricia proof against the header an attestation is checked under.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Proof {
    /// An account and one of its storage slots, under the state root.
    Storage(StorageProof),
    /// An account alone, under the state root.
    Account(AccountProof),
    /// One log of a transaction's receipt, under the receipts root.
    Receipt(ReceiptProof),
}

/// A check over the values one of the attestation's proofs establishes. `proof` is that proof's
/// position in [`Attestation::proofs`].
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Predicate {
    CheckSlotValue { proof: u8, op: Op, value: Root },
    CheckAccountBalance { proof: u8, op: Op, value: Root },
    CheckAccountNonce { proof: u8, op: Op, value: u64 },
    CheckAccountCode { proof: u8, code_hash: Root },
    CheckLogAddress { proof: u8, address: [u8; 20] },
    CheckLogTopic { proof: u8, index: u8, op: Op, value: Root },
    /// `word` is the index of a 32-byte word of the log's data.
    CheckLogData { proof: u8, word: u8, op: Op, value: Root },
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Attestation {
    /// The chain the consumer means; refused against another chain's record (F.5).
    pub chain_id: u64,
    pub proofs: Vec<Proof>,
    pub predicates: Vec<Predicate>,
}

/// Whether an ID is safe as a one-shot nullifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Class {
    /// The same state gives the same ID at any block: one-shot only for state written once.
    State,
    /// An event: one ID per occurrence.
    Occurrence,
}

/// What a statement says about one of the attestation's proofs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Fact {
    pub class: Class,
    pub id: Root,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Statement {
    /// The instance whose record the attestation was checked against (F.15).
    pub instance: AccountId,
    pub chain_id: u64,
    pub block_number: u64,
    /// One per proof, in the attestation's order.
    pub facts: Vec<Fact>,
    pub predicates: Vec<Predicate>,
}

/// A proof once verified: the Ethereum state it establishes.
enum Proven {
    Slot { address: [u8; 20], slot: Root, value: Root },
    Account { address: [u8; 20], nonce: u64, balance: Root, code_hash: Root },
    Event { tx_index: u64, log_index: u64, log: Log },
}

fn verify(header: &HeaderRecord, proof: &Proof) -> Result<Proven, Refusal> {
    Ok(match proof {
        Proof::Storage(p) => {
            mpt::verify_storage(&header.state_root, p)?;
            Proven::Slot { address: p.address, slot: p.slot, value: p.value }
        }
        Proof::Account(p) => {
            mpt::verify_account(&header.state_root, &p.address, &p.account, &p.account_proof)?;
            Proven::Account { address: p.address, nonce: p.account.nonce, balance: p.account.balance, code_hash: p.account.code_hash }
        }
        Proof::Receipt(p) => {
            let log = mpt::verify_receipt_log(&header.receipts_root, p)?;
            // A reverted transaction's logs were never applied: the event didn't happen.
            if !log.success {
                return Err(Refusal::ConditionUnsatisfied);
            }
            Proven::Event { tx_index: p.tx_index, log_index: p.log_index, log }
        }
    })
}

fn u64_le(n: u64) -> [u8; 8] {
    n.to_le_bytes()
}

impl Proven {
    /// `SHA-256(domain ‖ chain_id ‖ kind ‖ fields)`, every field fixed-width or length-prefixed,
    /// so distinct state never encodes the same (statements.md, section 3). Nothing about the
    /// predicate, the record used or the instance enters it.
    fn fact(&self, header: &HeaderRecord) -> Fact {
        let mut id = Sha256::new();
        id.update(ID_DOMAIN);
        id.update(u64_le(header.chain_id));
        let class = match self {
            Proven::Slot { address, slot, value } => {
                id.update([0]);
                id.update(address);
                id.update(slot);
                id.update(value);
                Class::State
            }
            Proven::Event { tx_index, log_index, log } => {
                id.update([1]);
                id.update(header.block_hash);
                id.update(u64_le(*tx_index));
                id.update(u64_le(*log_index));
                id.update(log.address);
                id.update(u64_le(log.topics.len() as u64));
                log.topics.iter().for_each(|t| id.update(t));
                id.update(u64_le(log.data.len() as u64));
                id.update(&log.data);
                Class::Occurrence
            }
            Proven::Account { address, nonce, balance, code_hash } => {
                id.update([2]);
                id.update(address);
                id.update(u64_le(*nonce));
                id.update(balance);
                id.update(code_hash);
                Class::State
            }
        };
        Fact { class, id: id.finalize().into() }
    }
}

/// Whether `predicate` holds of the proven state. A predicate over a value its proof doesn't
/// establish is malformed: it is refused, never read as zero.
fn holds(predicate: &Predicate, proven: &[Proven]) -> Result<bool, Refusal> {
    let at = |i: u8| proven.get(i as usize).ok_or(Refusal::MalformedInput);
    let word = |w: &Root| U256::from_be_bytes(*w);
    Ok(match predicate {
        Predicate::CheckSlotValue { proof, op, value } => match at(*proof)? {
            Proven::Slot { value: v, .. } => op.compare(word(v), word(value)),
            _ => return Err(Refusal::MalformedInput),
        },
        Predicate::CheckAccountBalance { proof, op, value } => match at(*proof)? {
            Proven::Account { balance, .. } => op.compare(word(balance), word(value)),
            _ => return Err(Refusal::MalformedInput),
        },
        Predicate::CheckAccountNonce { proof, op, value } => match at(*proof)? {
            Proven::Account { nonce, .. } => op.compare(U256::from(*nonce), U256::from(*value)),
            _ => return Err(Refusal::MalformedInput),
        },
        Predicate::CheckAccountCode { proof, code_hash } => match at(*proof)? {
            Proven::Account { code_hash: c, .. } => c == code_hash,
            _ => return Err(Refusal::MalformedInput),
        },
        Predicate::CheckLogAddress { proof, address } => match at(*proof)? {
            Proven::Event { log, .. } => &log.address == address,
            _ => return Err(Refusal::MalformedInput),
        },
        Predicate::CheckLogTopic { proof, index, op, value } => match at(*proof)? {
            Proven::Event { log, .. } => {
                let topic = log.topics.get(*index as usize).ok_or(Refusal::MalformedInput)?;
                op.compare(word(topic), word(value))
            }
            _ => return Err(Refusal::MalformedInput),
        },
        Predicate::CheckLogData { proof, word: w, op, value } => match at(*proof)? {
            Proven::Event { log, .. } => {
                let start = 32 * *w as usize;
                let chunk = log.data.get(start..start + 32).ok_or(Refusal::MalformedInput)?;
                op.compare(U256::from_be_slice(chunk), word(value))
            }
            _ => return Err(Refusal::MalformedInput),
        },
    })
}

/// Checks `attestation` against `header`, a record of `instance` or a block it reaches, and
/// returns the statement, or the refusal.
pub fn check(instance: &AccountId, header: &HeaderRecord, attestation: &Attestation) -> Result<Statement, Refusal> {
    if attestation.chain_id != header.chain_id {
        return Err(Refusal::WrongChain);
    }
    if attestation.proofs.is_empty() {
        return Err(Refusal::MalformedInput);
    }
    let proven = attestation.proofs.iter().map(|p| verify(header, p)).collect::<Result<Vec<_>, _>>()?;
    for predicate in &attestation.predicates {
        if !holds(predicate, &proven)? {
            return Err(Refusal::ConditionUnsatisfied);
        }
    }
    Ok(Statement {
        instance: instance.clone(),
        chain_id: header.chain_id,
        block_number: header.block_number,
        facts: proven.iter().map(|p| p.fact(header)).collect(),
        predicates: attestation.predicates.clone(),
    })
}
