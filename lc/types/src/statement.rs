//! What a consumer attests: a proof of Ethereum state or of a log, checked against a record,
//! and predicates over the values it proves. The result is a statement with a deterministic ID a
//! consumer can key a nullifier on (F.3, F.4, F.5, F.6).

use crate::error::Error;
use crate::mpt::{self, ReceiptProof, StorageProof};
use crate::{HeaderRecord, Root};
use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A value a proof establishes, compared as a 32-byte big-endian word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Field {
    Balance,
    Nonce,
    SlotValue,
    LogAddress,
    LogTopic(u8),
    /// The `i`-th 32-byte word of the log's data.
    LogData(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Predicate {
    pub field: Field,
    pub cmp: Cmp,
    pub value: Root,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Proof {
    Storage(StorageProof),
    Log(ReceiptProof),
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Query {
    /// The chain the consumer expects; refused if the record is another chain's (F.5).
    pub chain_id: u64,
    pub proof: Proof,
    /// All must hold: a compound condition over one header (soft #4).
    pub predicates: Vec<Predicate>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Statement {
    pub id: Root,
    pub chain_id: u64,
    pub block_number: u64,
    pub predicates: Vec<Predicate>,
}

fn word(b: &[u8]) -> Root {
    let mut w = [0u8; 32];
    let n = b.len().min(32);
    w[32 - n..].copy_from_slice(&b[b.len() - n..]);
    w
}

fn holds(v: &Root, p: &Predicate) -> bool {
    match p.cmp {
        Cmp::Eq => v == &p.value,
        Cmp::Ne => v != &p.value,
        Cmp::Lt => v < &p.value,
        Cmp::Le => v <= &p.value,
        Cmp::Gt => v > &p.value,
        Cmp::Ge => v >= &p.value,
    }
}

/// Verifies `q` against `r` and returns the statement, or the typed refusal.
///
/// Statement IDs carry a nullifier-safety class (proposal section 7). A **state** statement's ID
/// covers `(chain_id, address, slot, value)`: the same state is the same statement at any block,
/// as F.6 requires, so it is safe as a one-shot nullifier only for a write-once slot. A **log**
/// statement's ID covers `(chain_id, block_hash, tx_index, log_index, log)`: one per occurrence.
pub fn evaluate(r: &HeaderRecord, q: &Query) -> Result<Statement, Error> {
    if q.chain_id != r.chain_id {
        return Err(Error::ChainMismatch);
    }
    let mut id = Sha256::new();
    id.update(b"eth_lc/statement/v1");
    id.update(r.chain_id.to_le_bytes());
    let value: Box<dyn Fn(Field) -> Option<Root>> = match &q.proof {
        Proof::Storage(p) => {
            mpt::verify_storage(&r.state_root, p)?;
            id.update([0u8]);
            id.update(p.address);
            id.update(p.slot);
            id.update(p.value);
            let (balance, nonce, slot) = (p.account.balance, word(&p.account.nonce.to_be_bytes()), p.value);
            Box::new(move |f| match f {
                Field::Balance => Some(balance),
                Field::Nonce => Some(nonce),
                Field::SlotValue => Some(slot),
                _ => None,
            })
        }
        Proof::Log(p) => {
            let log = mpt::verify_receipt_log(&r.receipts_root, p)?;
            // A reverted transaction's logs prove nothing happened.
            if !log.success {
                return Err(Error::PredicateFalse);
            }
            id.update([1u8]);
            id.update(r.block_hash);
            id.update(p.tx_index.to_le_bytes());
            id.update(p.log_index.to_le_bytes());
            id.update(log.address);
            for t in &log.topics {
                id.update(t);
            }
            id.update(&log.data);
            Box::new(move |f| match f {
                Field::LogAddress => Some(word(&log.address)),
                Field::LogTopic(i) => log.topics.get(i as usize).copied(),
                Field::LogData(i) => log.data.get(32 * i as usize..32 * i as usize + 32).map(word),
                _ => None,
            })
        }
    };
    for p in &q.predicates {
        // A field the proof doesn't establish is refused, never read as zero.
        let v = value(p.field).ok_or(Error::Malformed)?;
        if !holds(&v, p) {
            return Err(Error::PredicateFalse);
        }
    }
    Ok(Statement { id: id.finalize().into(), chain_id: r.chain_id, block_number: r.block_number, predicates: q.predicates.clone() })
}
