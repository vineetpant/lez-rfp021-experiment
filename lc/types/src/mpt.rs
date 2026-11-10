//! Merkle-Patricia proofs against a verified execution root, with `alloy-trie`: an account, an
//! account and one of its storage slots (`eth_getProof`), and one log of a transaction receipt.
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

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Account {
    pub nonce: u64,
    pub balance: Root,
    pub storage_root: Root,
    pub code_hash: Root,
}

/// An account under a state root: the account half of an `eth_getProof` response.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct AccountProof {
    pub address: [u8; 20],
    pub account: Account,
    pub account_proof: Vec<Vec<u8>>,
}

/// An account and one storage slot under a state root, as `eth_getProof` returns them.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct StorageProof {
    pub address: [u8; 20],
    pub account: Account,
    pub account_proof: Vec<Vec<u8>>,
    pub slot: Root,
    /// The slot's value, big-endian.
    pub value: Root,
    pub storage_proof: Vec<Vec<u8>>,
}

fn nodes(proof: &[Vec<u8>]) -> Vec<Bytes> {
    proof.iter().map(|n| Bytes::copy_from_slice(n)).collect()
}

/// Checks that the state trie under `state_root` holds the account at `address` with exactly
/// these fields.
pub fn verify_account(state_root: &Root, address: &[u8; 20], account: &Account, proof: &[Vec<u8>]) -> Result<(), Refusal> {
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

/// Checks `p` under `state_root`: the account is in the state trie with exactly these fields,
/// and the slot holds exactly this value (a zero value is proven absent, as Ethereum stores it).
pub fn verify_storage(state_root: &Root, p: &StorageProof) -> Result<(), Refusal> {
    verify_account(state_root, &p.address, &p.account, &p.account_proof)?;

    let value = U256::from_be_bytes(p.value);
    let expected = (!value.is_zero()).then(|| {
        let mut v = Vec::new();
        value.encode(&mut v);
        v
    });
    let key = Nibbles::unpack(keccak256(p.slot));
    verify_proof(b256(&p.account.storage_root), key, expected, &nodes(&p.storage_proof)).map_err(|_| Refusal::InvalidInclusionProof)
}

/// One log of a receipt under a receipts root: the receipt, EIP-2718 encoded as it sits in the
/// trie, at transaction `tx_index`, and the position of the log in it.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ReceiptProof {
    pub tx_index: u64,
    pub receipt: Vec<u8>,
    pub proof: Vec<Vec<u8>>,
    pub log_index: u64,
}

/// What a receipt-log proof establishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Log {
    pub success: bool,
    pub address: [u8; 20],
    pub topics: Vec<Root>,
    pub data: Vec<u8>,
}

/// Checks `p` under `receipts_root` and returns the log it points at.
pub fn verify_receipt_log(receipts_root: &Root, p: &ReceiptProof) -> Result<Log, Refusal> {
    let mut index = Vec::new();
    p.tx_index.encode(&mut index);
    let key = Nibbles::unpack(&index);
    verify_proof(b256(receipts_root), key, Some(p.receipt.clone()), &nodes(&p.proof)).map_err(|_| Refusal::InvalidInclusionProof)?;
    let receipt = <ReceiptEnvelope as alloy_eips::eip2718::Decodable2718>::decode_2718(&mut p.receipt.as_slice())
        .map_err(|_| Refusal::MalformedInput)?;
    let log = receipt.logs().get(p.log_index as usize).ok_or(Refusal::MalformedInput)?;
    Ok(Log {
        success: receipt.status(),
        address: <Address as Into<[u8; 20]>>::into(log.address),
        topics: log.topics().iter().map(|t: &B256| t.0).collect(),
        data: log.data.data.to_vec(),
    })
}
