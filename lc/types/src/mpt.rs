//! Merkle-Patricia proofs against a verified execution root, with `alloy-trie`: an account and
//! one of its storage slots (`eth_getProof`), and one log of a transaction receipt.
//!
//! `alloy-trie` checks a proof for an expected value, so the prover supplies the value: the
//! account's fields, the slot's value, the receipt. The proof then establishes exactly that
//! value at that key, or fails.

use crate::error::Refusal;
use crate::{Root, b256};
use alloy_consensus::ReceiptEnvelope;
use alloy_primitives::{Address, B256, Bytes, U256, keccak256};
use alloy_rlp::Encodable;
use alloy_trie::{Nibbles, TrieAccount, proof::verify_proof};
use borsh::{BorshDeserialize, BorshSerialize};

/// An Ethereum account's fields, as the state trie holds them.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct EthAccount {
    pub nonce: u64,
    pub balance: Root,
    pub storage_root: Root,
    pub code_hash: Root,
}

fn nodes(proof: &[Vec<u8>]) -> Vec<Bytes> {
    proof.iter().map(|n| Bytes::copy_from_slice(n)).collect()
}

/// Checks that the state trie under `state_root` holds the account at `address` with exactly
/// these fields.
pub fn verify_account(state_root: &Root, address: &[u8; 20], account: &EthAccount, proof: &[Vec<u8>]) -> Result<(), Refusal> {
    let leaf = TrieAccount {
        nonce: account.nonce,
        balance: U256::from_be_bytes(account.balance),
        storage_root: b256(&account.storage_root),
        code_hash: b256(&account.code_hash),
    };
    let mut encoded = Vec::new();
    leaf.encode(&mut encoded);
    let key = Nibbles::unpack(keccak256(address));
    verify_proof(b256(state_root), key, Some(encoded), &nodes(proof)).map_err(|_| Refusal::InvalidInclusionProof)
}

/// Checks that `account`'s storage trie holds `value` at `slot` (a zero value is proven absent,
/// as Ethereum stores it).
pub fn verify_slot(account: &EthAccount, slot: &Root, value: &Root, proof: &[Vec<u8>]) -> Result<(), Refusal> {
    let value = U256::from_be_bytes(*value);
    let expected = (!value.is_zero()).then(|| {
        let mut v = Vec::new();
        value.encode(&mut v);
        v
    });
    let key = Nibbles::unpack(keccak256(slot));
    verify_proof(b256(&account.storage_root), key, expected, &nodes(proof)).map_err(|_| Refusal::InvalidInclusionProof)
}

/// What a receipt-log proof establishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Log {
    pub success: bool,
    pub address: [u8; 20],
    pub topics: Vec<Root>,
    pub data: Vec<u8>,
}

/// Checks that the receipts trie under `receipts_root` holds `receipt` (EIP-2718 encoded, as it
/// sits in the trie) at transaction `tx_index`, and returns its log at `log_index`.
pub fn verify_receipt_log(receipts_root: &Root, tx_index: u64, receipt: &[u8], proof: &[Vec<u8>], log_index: u64) -> Result<Log, Refusal> {
    let mut index = Vec::new();
    tx_index.encode(&mut index);
    let key = Nibbles::unpack(&index);
    verify_proof(b256(receipts_root), key, Some(receipt.to_vec()), &nodes(proof)).map_err(|_| Refusal::InvalidInclusionProof)?;
    let envelope = <ReceiptEnvelope as alloy_eips::eip2718::Decodable2718>::decode_2718(&mut &receipt[..])
        .map_err(|_| Refusal::MalformedInput)?;
    let log = envelope.logs().get(log_index as usize).ok_or(Refusal::MalformedInput)?;
    Ok(Log {
        success: envelope.status(),
        address: <Address as Into<[u8; 20]>>::into(log.address),
        topics: log.topics().iter().map(|t: &B256| t.0).collect(),
        data: log.data.data.to_vec(),
    })
}
