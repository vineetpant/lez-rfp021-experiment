//! Claims and the statements they produce (proposal sections 4.5–4.7).
//!
//! A consumer program sends a [`Claim`]: facts to prove against one finalised header, each a
//! Merkle-Patricia proof with the conditions its values must meet, all of which must hold (F.3,
//! F.4, soft #4). [`check`] verifies every proof, evaluates every condition and returns a
//! [`Statement`] carrying, for each fact, an identifier derived from the proven Ethereum state
//! alone, for a consumer to key its nullifier on (F.5, F.6, F.15).

use crate::error::Refusal;
use crate::mpt::{self, EthAccount, Log};
use crate::{FinalisedHeader, Root};
use alloy_primitives::U256;
use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::account::AccountId;
use sha2::{Digest, Sha256};

/// Domain of every statement ID; a new encoding gets a new version.
const ID_DOMAIN: &[u8] = b"eth_lc/statement/v1";

/// A value a fact establishes, read as a 32-byte big-endian word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Subject {
    AccountBalance,
    AccountNonce,
    AccountCodeHash,
    /// A field packed into the slot: `width` bytes, `offset` bytes up from the low-order end, as
    /// Solidity packs them. `{ offset: 0, width: 32 }` is the whole slot.
    StorageValue { offset: u8, width: u8 },
    LogEmitter,
    LogTopic(u8),
    /// The `i`-th 32-byte word of the log's data.
    LogDataWord(u8),
    LogTopicCount,
    LogDataLength,
}

/// How a subject compares with the consumer's value, as unsigned 256-bit integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Op {
    Equal,
    NotEqual,
    Less,
    AtMost,
    Greater,
    AtLeast,
}

/// `subject op rhs` must hold. `rhs` is a uint, an int (two's complement), an address or a
/// bytes32, as a 32-byte word.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Condition {
    pub subject: Subject,
    pub op: Op,
    pub rhs: Root,
}

/// What is proven, against the header a claim is checked under.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Proof {
    /// One log of a transaction's receipt, under the receipts root.
    Log { tx_index: u64, receipt: Vec<u8>, proof: Vec<Vec<u8>>, log_index: u64 },
    /// An account and one of its storage slots, under the state root.
    Storage { address: [u8; 20], account: EthAccount, account_proof: Vec<Vec<u8>>, slot: Root, value: Root, storage_proof: Vec<Vec<u8>> },
    /// An account alone, under the state root.
    Account { address: [u8; 20], account: EthAccount, account_proof: Vec<Vec<u8>> },
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Fact {
    pub proof: Proof,
    /// What must hold of the values the proof establishes.
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Claim {
    /// The chain the consumer expects; refused against another chain's header (F.5).
    pub chain_id: u64,
    /// Each proven under the same header; all must hold.
    pub facts: Vec<Fact>,
}

/// Whether a fact's ID is safe as a one-shot nullifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Class {
    /// Logs: one ID per event.
    Occurrence,
    /// Storage and accounts: the same state gives the same ID at any block, so one-shot only
    /// for state written once.
    State,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ProvenFact {
    pub class: Class,
    pub id: Root,
    /// What was asserted of it.
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Statement {
    /// The instance whose header was used (F.15).
    pub instance: AccountId,
    pub chain_id: u64,
    /// The block the facts were proven under.
    pub block_number: u64,
    /// One per fact, in the claim's order.
    pub facts: Vec<ProvenFact>,
}

/// A proof once verified: the Ethereum state it establishes.
enum Established {
    Slot { address: [u8; 20], slot: Root, value: Root },
    Account { address: [u8; 20], account: EthAccount },
    Event { tx_index: u64, log_index: u64, log: Log },
}

fn establish(header: &FinalisedHeader, proof: &Proof) -> Result<Established, Refusal> {
    Ok(match proof {
        Proof::Storage { address, account, account_proof, slot, value, storage_proof } => {
            mpt::verify_account(&header.state_root, address, account, account_proof)?;
            mpt::verify_slot(account, slot, value, storage_proof)?;
            Established::Slot { address: *address, slot: *slot, value: *value }
        }
        Proof::Account { address, account, account_proof } => {
            mpt::verify_account(&header.state_root, address, account, account_proof)?;
            Established::Account { address: *address, account: account.clone() }
        }
        Proof::Log { tx_index, receipt, proof, log_index } => {
            let log = mpt::verify_receipt_log(&header.receipts_root, *tx_index, receipt, proof, *log_index)?;
            // A reverted transaction's logs were never applied: the event didn't happen.
            if !log.success {
                return Err(Refusal::ConditionUnsatisfied);
            }
            Established::Event { tx_index: *tx_index, log_index: *log_index, log }
        }
    })
}

fn le(n: u64) -> [u8; 8] {
    n.to_le_bytes()
}

/// `n` as a 32-byte big-endian word.
fn number(n: u64) -> Root {
    U256::from(n).to_be_bytes()
}

impl Established {
    /// `subject`'s value, or `MalformedInput` if this fact doesn't establish it: never zero.
    fn read(&self, subject: Subject) -> Result<Root, Refusal> {
        let mut word = [0u8; 32];
        match (self, subject) {
            (Established::Account { account, .. }, Subject::AccountBalance) => Ok(account.balance),
            (Established::Account { account, .. }, Subject::AccountNonce) => Ok(number(account.nonce)),
            (Established::Account { account, .. }, Subject::AccountCodeHash) => Ok(account.code_hash),
            (Established::Slot { value, .. }, Subject::StorageValue { offset, width }) => {
                let (offset, width) = (offset as usize, width as usize);
                if width == 0 || offset + width > 32 {
                    return Err(Refusal::MalformedInput);
                }
                word[32 - width..].copy_from_slice(&value[32 - offset - width..32 - offset]);
                Ok(word)
            }
            (Established::Event { log, .. }, Subject::LogEmitter) => {
                word[12..].copy_from_slice(&log.address);
                Ok(word)
            }
            (Established::Event { log, .. }, Subject::LogTopic(i)) => log.topics.get(i as usize).copied().ok_or(Refusal::MalformedInput),
            (Established::Event { log, .. }, Subject::LogDataWord(i)) => {
                let start = 32 * i as usize;
                log.data.get(start..start + 32).map(|w| w.try_into().expect("32 bytes")).ok_or(Refusal::MalformedInput)
            }
            (Established::Event { log, .. }, Subject::LogTopicCount) => Ok(number(log.topics.len() as u64)),
            (Established::Event { log, .. }, Subject::LogDataLength) => Ok(number(log.data.len() as u64)),
            _ => Err(Refusal::MalformedInput),
        }
    }

    /// `SHA-256(domain ‖ chain_id ‖ kind ‖ fields)`, every field fixed-width or length-prefixed,
    /// so distinct state never encodes the same. Nothing about the conditions, the header used
    /// or the instance enters it, except the log's block hash, which places the event.
    fn identify(&self, header: &FinalisedHeader) -> (Class, Root) {
        let mut id = Sha256::new();
        id.update(ID_DOMAIN);
        id.update(le(header.chain.chain_id));
        let class = match self {
            Established::Slot { address, slot, value } => {
                id.update([0]);
                id.update(address);
                id.update(slot);
                id.update(value);
                Class::State
            }
            Established::Event { tx_index, log_index, log } => {
                id.update([1]);
                id.update(header.block_hash);
                id.update(le(*tx_index));
                id.update(le(*log_index));
                id.update(log.address);
                id.update(le(log.topics.len() as u64));
                log.topics.iter().for_each(|t| id.update(t));
                id.update(le(log.data.len() as u64));
                id.update(&log.data);
                Class::Occurrence
            }
            Established::Account { address, account } => {
                id.update([2]);
                id.update(address);
                id.update(le(account.nonce));
                id.update(account.balance);
                id.update(account.code_hash);
                Class::State
            }
        };
        (class, id.finalize().into())
    }
}

/// Checks `claim` against `header`, a finalised header of `instance` or a block it reaches, and
/// returns the statement, or the refusal.
pub fn check(instance: &AccountId, header: &FinalisedHeader, claim: &Claim) -> Result<Statement, Refusal> {
    if claim.chain_id != header.chain.chain_id {
        return Err(Refusal::WrongChain);
    }
    if claim.facts.is_empty() {
        return Err(Refusal::MalformedInput);
    }
    let mut facts = Vec::with_capacity(claim.facts.len());
    for fact in &claim.facts {
        let established = establish(header, &fact.proof)?;
        for c in &fact.conditions {
            let value = established.read(c.subject)?;
            let holds = match c.op {
                Op::Equal => value == c.rhs,
                Op::NotEqual => value != c.rhs,
                Op::Less => U256::from_be_bytes(value) < U256::from_be_bytes(c.rhs),
                Op::AtMost => U256::from_be_bytes(value) <= U256::from_be_bytes(c.rhs),
                Op::Greater => U256::from_be_bytes(value) > U256::from_be_bytes(c.rhs),
                Op::AtLeast => U256::from_be_bytes(value) >= U256::from_be_bytes(c.rhs),
            };
            if !holds {
                return Err(Refusal::ConditionUnsatisfied);
            }
        }
        let (class, id) = established.identify(header);
        facts.push(ProvenFact { class, id, conditions: fact.conditions.clone() });
    }
    Ok(Statement { instance: instance.clone(), chain_id: header.chain.chain_id, block_number: header.block_number, facts })
}
