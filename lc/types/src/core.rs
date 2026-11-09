//! The light-client checks the program runs: the Altair light-client sync protocol, as the
//! Ethereum consensus specs define it (`validate_light_client_update`,
//! `apply_light_client_update`), for finalized headers only, with BLS through `blst`.
//!
//! It is written for cost inside the zkVM. The signature check uses `blst` with RISC Zero's
//! accelerator, and the signing key is the committee's aggregate key less the few members that
//! didn't sign, so only those keys are decompressed. Committee roots are hashed directly over the
//! keys' SSZ bytes. The program receives these compact types, never JSON.
//!
//! Helios's `consensus-core` checks the same updates in the tests (`tests/oracle.rs`): on every
//! recorded mainnet update and every adversarial mutation, both must reach the same verdict and
//! the same store.
//!
//! On top of the spec, an update must be signed by at least 2/3 of the committee
//! (`BelowParticipation`), because only finalized headers are applied and the spec applies those
//! only with a supermajority; and it must advance the store (`AlreadyApplied`, R.8).

use crate::error::Refusal;
use crate::{ChainConfig, Root};
use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};

pub const SLOTS_PER_EPOCH: u64 = 32;
pub const SLOTS_PER_PERIOD: u64 = 8192;
pub const COMMITTEE: usize = 512;
/// A committee's SSZ: 512 keys of 48 bytes, then the aggregate key.
pub const COMMITTEE_BYTES: usize = (COMMITTEE + 1) * 48;
const DOMAIN_SYNC_COMMITTEE: [u8; 4] = [7, 0, 0, 0];
const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

pub fn period(slot: u64) -> u64 {
    slot / SLOTS_PER_PERIOD
}

#[derive(Clone, Debug, Default, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct BeaconHeader {
    pub slot: u64,
    pub proposer_index: u64,
    pub parent_root: Root,
    pub state_root: Root,
    pub body_root: Root,
}

/// The execution payload header from Deneb on (Deneb, Electra and Fulu share its shape).
#[derive(Clone, Debug, Default, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct PayloadHeader {
    pub parent_hash: Root,
    pub fee_recipient: [u8; 20],
    pub state_root: Root,
    pub receipts_root: Root,
    pub logs_bloom: Vec<u8>,
    pub prev_randao: Root,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: Vec<u8>,
    /// A uint256, little-endian, as SSZ lays it out.
    pub base_fee_per_gas: Root,
    pub block_hash: Root,
    pub transactions_root: Root,
    pub withdrawals_root: Root,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

/// What a light-client header commits about its execution block.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Execution {
    /// Deneb to Fulu: the payload header, under the body root.
    Payload(PayloadHeader),
    /// Gloas: only the execution block hash.
    BlockHash(Root),
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Header {
    pub beacon: BeaconHeader,
    pub execution: Execution,
    pub execution_branch: Vec<Root>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Bootstrap {
    pub header: Header,
    pub committee_branch: Vec<Root>,
}

/// A light-client update, or a finality update (no next committee).
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Update {
    pub attested: Header,
    pub finalized: Header,
    pub finality_branch: Vec<Root>,
    /// The next committee's keys as SSZ, and its branch under the attested state.
    pub next_committee: Option<(Vec<u8>, Vec<Root>)>,
    pub bits: [u8; 64],
    pub signature: [u8; 96],
    pub signature_slot: u64,
}

/// The light-client store an instance keeps. Committees are kept as their SSZ roots.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Store {
    pub finalized: BeaconHeader,
    pub current_committee: Root,
    pub next_committee: Option<Root>,
}

// ---- SSZ ----

fn sha256(a: &[u8], b: &[u8]) -> Root {
    let mut h = Sha256::new();
    h.update(a);
    h.update(b);
    h.finalize().into()
}

fn chunk(bytes: &[u8]) -> Root {
    let mut c = [0u8; 32];
    c[..bytes.len()].copy_from_slice(bytes);
    c
}

fn uint(v: u64) -> Root {
    chunk(&v.to_le_bytes())
}

/// The root of `leaves`, padded with zero chunks to `width`, a power of two.
fn merkleize(leaves: &[Root], width: usize) -> Root {
    let mut layer: Vec<Root> = leaves.to_vec();
    layer.resize(width, [0; 32]);
    while layer.len() > 1 {
        layer = layer.chunks(2).map(|p| sha256(&p[0], &p[1])).collect();
    }
    layer[0]
}

pub fn beacon_root(h: &BeaconHeader) -> Root {
    merkleize(&[uint(h.slot), uint(h.proposer_index), h.parent_root, h.state_root, h.body_root], 8)
}

pub fn payload_root(p: &PayloadHeader) -> Root {
    let bloom: Vec<Root> = p.logs_bloom.chunks(32).map(chunk).collect();
    // `extra_data` is a ByteList[32]: one chunk, with its length mixed in.
    let extra = sha256(&chunk(&p.extra_data), &uint(p.extra_data.len() as u64));
    let fields = [
        p.parent_hash,
        chunk(&p.fee_recipient),
        p.state_root,
        p.receipts_root,
        merkleize(&bloom, 8),
        p.prev_randao,
        uint(p.block_number),
        uint(p.gas_limit),
        uint(p.gas_used),
        uint(p.timestamp),
        extra,
        p.base_fee_per_gas,
        p.block_hash,
        p.transactions_root,
        p.withdrawals_root,
        uint(p.blob_gas_used),
        uint(p.excess_blob_gas),
    ];
    merkleize(&fields, 32)
}

/// The SSZ root of a committee, from its SSZ bytes.
pub fn committee_root(bytes: &[u8]) -> Result<Root, Refusal> {
    if bytes.len() != COMMITTEE_BYTES {
        return Err(Refusal::MalformedInput);
    }
    let key = |k: &[u8]| sha256(&k[..32], &chunk(&k[32..48]));
    let keys: Vec<Root> = bytes[..COMMITTEE * 48].chunks(48).map(key).collect();
    Ok(sha256(&merkleize(&keys, COMMITTEE), &key(&bytes[COMMITTEE * 48..])))
}

/// Whether `branch` proves `leaf` at (`depth`, `index`) under `root`. A branch from a fork with a
/// deeper tree may carry leading zero nodes, as the spec allows.
pub fn is_valid_branch(leaf: Root, branch: &[Root], depth: usize, index: u64, root: &Root) -> bool {
    let Some(extra) = branch.len().checked_sub(depth) else { return false };
    if branch[..extra].iter().any(|n| n != &[0; 32]) {
        return false;
    }
    let node = branch[extra..]
        .iter()
        .enumerate()
        .fold(leaf, |node, (i, sib)| if (index >> i) & 1 == 1 { sha256(sib, &node) } else { sha256(&node, sib) });
    &node == root
}

// ---- the protocol ----

fn epoch(slot: u64) -> u64 {
    slot / SLOTS_PER_EPOCH
}

/// `is_valid_light_client_header`: the execution commitment under the body root.
pub fn is_valid_header(cfg: &ChainConfig, h: &Header) -> bool {
    let gloas = epoch(h.beacon.slot) >= cfg.gloas_epoch;
    match (&h.execution, gloas) {
        (Execution::Payload(p), false) => is_valid_branch(payload_root(p), &h.execution_branch, 4, 9, &h.beacon.body_root),
        (Execution::BlockHash(hash), true) => is_valid_branch(*hash, &h.execution_branch, 11, 808, &h.beacon.body_root),
        _ => false,
    }
}

/// Where a state field sits, by the fork of the state: (index, depth) for the finalized
/// checkpoint root, the current committee, and the next committee.
fn state_positions(cfg: &ChainConfig, slot: u64) -> [(u64, usize); 3] {
    let e = epoch(slot);
    if e >= cfg.gloas_epoch {
        [(223, 9), (897, 11), (898, 11)]
    } else if e >= cfg.electra_epoch {
        [(41, 7), (22, 6), (23, 6)]
    } else {
        [(41, 6), (22, 5), (23, 5)]
    }
}

/// `Init`: the store of an instance from a bootstrap whose header hashes to `checkpoint`, and the
/// bootstrap committee's keys.
pub fn bootstrap(cfg: &ChainConfig, checkpoint: &Root, b: &Bootstrap, committee: &[u8]) -> Result<Store, Refusal> {
    if beacon_root(&b.header.beacon) != *checkpoint {
        return Err(Refusal::CheckpointRootMismatch);
    }
    if !is_valid_header(cfg, &b.header) {
        return Err(Refusal::InvalidExecutionProof);
    }
    let root = committee_root(committee)?;
    let (index, depth) = state_positions(cfg, b.header.beacon.slot)[1];
    if !is_valid_branch(root, &b.committee_branch, depth, index, &b.header.beacon.state_root) {
        return Err(Refusal::InvalidCommitteeProof);
    }
    Ok(Store { finalized: b.header.beacon.clone(), current_committee: root, next_committee: None })
}

pub fn signers(bits: &[u8; 64]) -> usize {
    bits.iter().map(|b| b.count_ones() as usize).sum()
}

fn bit(bits: &[u8; 64], i: usize) -> bool {
    bits[i / 8] >> (i % 8) & 1 == 1
}

/// The signing root of `header` for a sync-committee signature at `signature_slot`.
pub fn signing_root(cfg: &ChainConfig, header: &BeaconHeader, signature_slot: u64) -> Root {
    let version = cfg.fork_version(epoch(signature_slot.max(1) - 1));
    let fork_data_root = sha256(&chunk(&version), &cfg.genesis_validators_root);
    let mut domain = [0u8; 32];
    domain[..4].copy_from_slice(&DOMAIN_SYNC_COMMITTEE);
    domain[4..].copy_from_slice(&fork_data_root[..28]);
    sha256(&beacon_root(header), &domain)
}

/// Whether `committee` (its SSZ keys) signed `msg` with `signature`, by the members `bits` marks.
/// The key is the committee's aggregate key less the members that didn't sign; a supermajority
/// signs, so only a few keys are decompressed.
pub fn verify_signature(committee: &[u8], bits: &[u8; 64], msg: &Root, signature: &[u8; 96]) -> bool {
    use blst::*;
    let key = |i: usize| -> Option<blst_p1_affine> {
        let mut p = blst_p1_affine::default();
        (unsafe { blst_p1_uncompress(&mut p, committee[i * 48..i * 48 + 48].as_ptr()) } == BLST_ERROR::BLST_SUCCESS).then_some(p)
    };
    let Some(aggregate) = key(COMMITTEE) else { return false };
    let mut sum = blst_p1::default();
    unsafe { blst_p1_from_affine(&mut sum, &aggregate) };
    for i in (0..COMMITTEE).filter(|i| !bit(bits, *i)) {
        let Some(k) = key(i) else { return false };
        let mut neg = blst_p1::default();
        unsafe {
            blst_p1_from_affine(&mut neg, &k);
            blst_p1_cneg(&mut neg, true);
            blst_p1_add_or_double(&mut sum, &sum, &neg);
        }
    }
    let mut signing_key = blst_p1_affine::default();
    unsafe { blst_p1_to_affine(&mut signing_key, &sum) };
    let pk = min_pk::PublicKey::from(signing_key);
    let Ok(sig) = min_pk::Signature::from_bytes(signature) else { return false };
    // The signature is checked to be in G2; the key is a sum of keys whose committee root the
    // instance verified, so it isn't re-validated.
    sig.verify(true, msg, DST, &[], &pk, false) == BLST_ERROR::BLST_SUCCESS
}

/// `validate_light_client_update` then `apply_light_client_update`, for a finalized header: the
/// store after `u`, and whether it finalized a newer header. `current` and `next` are the keys of
/// the committees `store` holds the roots of.
pub fn update(cfg: &ChainConfig, store: &Store, u: &Update, current: &[u8], next: Option<&[u8]>) -> Result<(Store, bool), Refusal> {
    // Past the weak-subjectivity bound the instance is replaced, not advanced (F.10).
    if cfg.slot_time(u.signature_slot) > cfg.slot_time(store.finalized.slot) + cfg.max_store_age {
        return Err(Refusal::StaleStore);
    }
    if signers(&u.bits) * 3 < COMMITTEE * 2 {
        return Err(Refusal::InsufficientSigners);
    }
    if !(u.signature_slot > u.attested.beacon.slot && u.attested.beacon.slot >= u.finalized.beacon.slot) {
        return Err(Refusal::MalformedInput);
    }
    let store_period = period(store.finalized.slot);
    let signature_period = period(u.signature_slot);
    let known_next = store.next_committee.is_some();
    if !(signature_period == store_period || (known_next && signature_period == store_period + 1)) {
        return Err(Refusal::UnverifiedHandoff);
    }
    let attested_period = period(u.attested.beacon.slot);
    let learns_next = !known_next && u.next_committee.is_some() && attested_period == store_period;
    if u.finalized.beacon.slot <= store.finalized.slot && !learns_next {
        return Err(Refusal::UpdateNotNewer);
    }

    if !is_valid_header(cfg, &u.attested) || !is_valid_header(cfg, &u.finalized) {
        return Err(Refusal::InvalidExecutionProof);
    }
    let [finality, _, next_position] = state_positions(cfg, u.attested.beacon.slot);
    if !is_valid_branch(beacon_root(&u.finalized.beacon), &u.finality_branch, finality.1, finality.0, &u.attested.beacon.state_root) {
        return Err(Refusal::UnfinalizedHeader);
    }
    let update_next = match &u.next_committee {
        None => None,
        Some((keys, branch)) => {
            let root = committee_root(keys)?;
            if !is_valid_branch(root, branch, next_position.1, next_position.0, &u.attested.beacon.state_root) {
                return Err(Refusal::InvalidCommitteeProof);
            }
            if attested_period == store_period && store.next_committee.is_some_and(|known| known != root) {
                return Err(Refusal::UnverifiedHandoff);
            }
            Some(root)
        }
    };

    // The committee that signs, checked against the root the store holds.
    let (signer, signer_root) = if signature_period == store_period {
        (current, store.current_committee)
    } else {
        (next.ok_or(Refusal::CommitteeKeysMismatch)?, store.next_committee.ok_or(Refusal::UnverifiedHandoff)?)
    };
    if committee_root(signer)? != signer_root {
        return Err(Refusal::CommitteeKeysMismatch);
    }
    if !verify_signature(signer, &u.bits, &signing_root(cfg, &u.attested.beacon, u.signature_slot), &u.signature) {
        return Err(Refusal::InvalidSignature);
    }

    // `apply_light_client_update`.
    let finalized_period = period(u.finalized.beacon.slot);
    let mut s = store.clone();
    if !known_next {
        if finalized_period != store_period {
            return Err(Refusal::UnverifiedHandoff);
        }
        s.next_committee = update_next;
    } else if finalized_period == store_period + 1 {
        s.current_committee = store.next_committee.expect("known");
        s.next_committee = if attested_period == finalized_period { update_next } else { None };
    }
    let advanced = u.finalized.beacon.slot > store.finalized.slot;
    if advanced {
        s.finalized = u.finalized.beacon.clone();
    }
    if s == *store {
        return Err(Refusal::UpdateNotNewer);
    }
    Ok((s, advanced))
}
