//! What a consumer attests: a claim about Ethereum, made of a fact (a proof of state or of a log,
//! checked against a record) and conditions on the values the fact establishes. A claim that
//! holds becomes a statement, with a deterministic ID a consumer can key a nullifier on (F.3, F.4,
//! F.5, F.6).

use crate::error::Refusal;
use crate::mpt::{self, Log, ReceiptProof, StorageProof};
use crate::{HeaderRecord, Root};
use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};

/// How a value is compared, as a 32-byte big-endian word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Op {
    Equal,
    NotEqual,
    Less,
    AtMost,
    Greater,
    AtLeast,
}

/// A value a fact establishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Subject {
    AccountBalance,
    AccountNonce,
    StorageValue,
    LogEmitter,
    LogTopic(u8),
    /// The `i`-th 32-byte word of the log's data.
    LogDataWord(u8),
}

/// `subject op rhs` must hold.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Condition {
    pub subject: Subject,
    pub op: Op,
    pub rhs: Root,
}

/// What is proven about Ethereum under a record.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Fact {
    /// An account and one of its storage slots, under the state root.
    Storage(StorageProof),
    /// One log of a transaction receipt, under the receipts root.
    Log(ReceiptProof),
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Claim {
    /// The chain the consumer expects; refused if the record is another chain's (F.5).
    pub chain_id: u64,
    pub fact: Fact,
    /// All must hold: a compound condition over one header (soft #4).
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Statement {
    pub id: Root,
    pub chain_id: u64,
    pub block_number: u64,
    pub conditions: Vec<Condition>,
}

fn word(b: &[u8]) -> Root {
    let mut w = [0u8; 32];
    let n = b.len().min(32);
    w[32 - n..].copy_from_slice(&b[b.len() - n..]);
    w
}

impl Op {
    fn holds(self, lhs: &Root, rhs: &Root) -> bool {
        match self {
            Self::Equal => lhs == rhs,
            Self::NotEqual => lhs != rhs,
            Self::Less => lhs < rhs,
            Self::AtMost => lhs <= rhs,
            Self::Greater => lhs > rhs,
            Self::AtLeast => lhs >= rhs,
        }
    }
}

/// A verified fact: the values it establishes, and the material its statement ID covers.
enum Proven<'a> {
    Storage(&'a StorageProof),
    Log(&'a ReceiptProof, Log),
}

impl Proven<'_> {
    /// `subject`'s value, or `None` if this kind of fact doesn't establish it.
    fn read(&self, subject: Subject) -> Option<Root> {
        match (self, subject) {
            (Self::Storage(p), Subject::AccountBalance) => Some(p.account.balance),
            (Self::Storage(p), Subject::AccountNonce) => Some(word(&p.account.nonce.to_be_bytes())),
            (Self::Storage(p), Subject::StorageValue) => Some(p.value),
            (Self::Log(_, l), Subject::LogEmitter) => Some(word(&l.address)),
            (Self::Log(_, l), Subject::LogTopic(i)) => l.topics.get(i as usize).copied(),
            (Self::Log(_, l), Subject::LogDataWord(i)) => l.data.get(32 * i as usize..32 * i as usize + 32).map(word),
            _ => None,
        }
    }

    /// The statement ID. A **state** fact's ID covers `(chain_id, address, slot, value)`: the same
    /// state is the same statement at any block, as F.6 requires, so it is safe as a one-shot
    /// nullifier only for a write-once slot. A **log** fact's ID covers `(chain_id, block_hash,
    /// tx_index, log_index, log)`: one per occurrence.
    fn id(&self, r: &HeaderRecord) -> Root {
        let mut h = Sha256::new();
        h.update(b"eth_lc/statement/v1");
        h.update(r.chain_id.to_le_bytes());
        match self {
            Self::Storage(p) => {
                h.update([0u8]);
                h.update(p.address);
                h.update(p.slot);
                h.update(p.value);
            }
            Self::Log(p, l) => {
                h.update([1u8]);
                h.update(r.block_hash);
                h.update(p.tx_index.to_le_bytes());
                h.update(p.log_index.to_le_bytes());
                h.update(l.address);
                l.topics.iter().for_each(|t| h.update(t));
                h.update(&l.data);
            }
        }
        h.finalize().into()
    }
}

/// Checks `claim` against `record` and returns the statement, or the refusal.
pub fn check(record: &HeaderRecord, claim: &Claim) -> Result<Statement, Refusal> {
    if claim.chain_id != record.chain_id {
        return Err(Refusal::WrongChain);
    }
    let proven = match &claim.fact {
        Fact::Storage(p) => {
            mpt::verify_storage(&record.state_root, p)?;
            Proven::Storage(p)
        }
        Fact::Log(p) => {
            let log = mpt::verify_receipt_log(&record.receipts_root, p)?;
            // A reverted transaction's logs prove nothing happened.
            if !log.success {
                return Err(Refusal::ConditionUnsatisfied);
            }
            Proven::Log(p, log)
        }
    };
    for c in &claim.conditions {
        // A value the fact doesn't establish is refused, never read as zero.
        let lhs = proven.read(c.subject).ok_or(Refusal::MalformedInput)?;
        if !c.op.holds(&lhs, &c.rhs) {
            return Err(Refusal::ConditionUnsatisfied);
        }
    }
    Ok(Statement { id: proven.id(record), chain_id: record.chain_id, block_number: record.block_number, conditions: claim.conditions.clone() })
}
