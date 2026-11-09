//! Helios's consensus checks, for an instance that keeps committee roots instead of keys.
//!
//! Helios verifies an update against a `LightClientStore` holding both 512-key committees. An
//! instance keeps only their 32-byte SSZ roots, so the submitter supplies the keys, these
//! functions check them against the roots, and build Helios's store from them for the length
//! of one check. On top of Helios they enforce three things:
//!
//! - **2/3 of the committee signed.** Helios verifies an update with one signer and, below 2/3,
//!   skips applying it without saying so. Here that is `BelowParticipation`.
//! - **The update advances the store**, or it is `AlreadyApplied` (R.8).
//! - **The store is fresh**: an update signed after the store went stale is `CheckpointStale`.
//!
//! Two things are done here rather than through Helios, both for cost inside the zkVM, where
//! Helios's generic SSZ hashing spends about 55k cycles a key:
//!
//! - **Committee roots** are computed directly over the keys' SSZ bytes with the accelerated
//!   SHA-256 ([`committee_root`]), once per committee; the native tests check they equal Helios's.
//! - **The bootstrap** is checked here ([`bootstrap`]): the header against the checkpoint, its
//!   execution branch, and the committee's branch, at the positions Helios uses. Through Helios it
//!   costs over 20M cycles, twice what one public transaction may use.

use crate::error::Error;
use crate::{ChainConfig, HeaderRecord, Root, Store, b256};
use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rlp::Decodable;
use sha2::{Digest, Sha256};
use ssz::{Decode, Encode};
use helios_consensus_core::consensus_spec::MainnetConsensusSpec as Spec;
use helios_consensus_core::types::{Bootstrap, FinalityUpdate, Forks, LightClientHeader, LightClientStore, SyncCommittee, Update};
use helios_consensus_core::{apply_finality_update, apply_update, verify_finality_update, verify_update};
use tree_hash::TreeHash;

pub type Committee = SyncCommittee<Spec>;

/// What an `Update` message carries: a period's best update, which can hand off the next
/// committee, or a finality update, which only advances the finalized header.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum AnyUpdate {
    Update(Update<Spec>),
    Finality(FinalityUpdate<Spec>),
}

impl AnyUpdate {
    fn signature_slot(&self) -> u64 {
        match self {
            Self::Update(u) => *u.signature_slot(),
            Self::Finality(u) => *u.signature_slot(),
        }
    }

    fn signers(&self) -> u64 {
        let bits = match self {
            Self::Update(u) => &u.sync_aggregate().sync_committee_bits,
            Self::Finality(u) => &u.sync_aggregate().sync_committee_bits,
        };
        bits.num_set_bits() as u64
    }
}

const COMMITTEE: u64 = 512;

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_cbor::from_slice(bytes).map_err(|_| Error::Malformed)
}

fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_cbor::to_vec(value).expect("CBOR encodes")
}

/// The record of a verified finalized `header`. Before Gloas its execution roots are in the
/// payload header Helios proved under the body root; from Gloas only the block hash is, and
/// `rlp`, the execution header that must hash to it, gives the roots.
pub fn record_of(cfg: &ChainConfig, header: &LightClientHeader, rlp: Option<&[u8]>, signers: u16) -> Result<HeaderRecord, Error> {
    let beacon = header.beacon();
    let (block_number, block_hash, state_root, receipts_root) = match header {
        LightClientHeader::Gloas(h) => {
            let rlp = rlp.ok_or(Error::BadExecutionBranch)?;
            if alloy_primitives::keccak256(rlp) != h.execution_block_hash {
                return Err(Error::BadExecutionBranch);
            }
            let e = Header::decode(&mut &rlp[..]).map_err(|_| Error::BadExecutionBranch)?;
            (e.number, h.execution_block_hash, e.state_root, e.receipts_root)
        }
        _ => {
            let e = header.execution().map_err(|_| Error::BadExecutionBranch)?;
            (*e.block_number(), *e.block_hash(), *e.state_root(), *e.receipts_root())
        }
    };
    Ok(HeaderRecord {
        chain_id: cfg.chain_id,
        slot: beacon.slot,
        beacon_root: beacon.tree_hash_root().0,
        block_number,
        block_hash: block_hash.0,
        state_root: state_root.0,
        receipts_root: receipts_root.0,
        signers,
    })
}

fn sha256(a: &[u8], b: &[u8]) -> Root {
    let mut h = Sha256::new();
    h.update(a);
    h.update(b);
    h.finalize().into()
}

/// The SSZ root of a sync committee, from its SSZ bytes: 512 keys of 48 bytes, then the
/// aggregate key. Each key's root is its two 32-byte chunks hashed; the keys merkleize into a
/// depth-9 tree; the container is that root and the aggregate's.
fn committee_root_ssz(bytes: &[u8]) -> Result<Root, Error> {
    if bytes.len() != (COMMITTEE as usize + 1) * 48 {
        return Err(Error::Malformed);
    }
    let key = |k: &[u8]| {
        let mut second = [0u8; 32];
        second[..16].copy_from_slice(&k[32..48]);
        sha256(&k[..32], &second)
    };
    let mut layer: Vec<Root> = bytes[..COMMITTEE as usize * 48].chunks(48).map(key).collect();
    while layer.len() > 1 {
        layer = layer.chunks(2).map(|p| sha256(&p[0], &p[1])).collect();
    }
    Ok(sha256(&layer[0], &key(&bytes[COMMITTEE as usize * 48..])))
}

/// The SSZ root of a committee.
pub fn committee_root(c: &Committee) -> Root {
    committee_root_ssz(&c.as_ssz_bytes()).expect("a committee encodes to 512 keys and an aggregate")
}

/// A committee's keys as `Update` and `Init` carry them: SSZ.
pub fn committee_bytes(c: &Committee) -> Vec<u8> {
    c.as_ssz_bytes()
}

/// Whether `branch` proves `leaf` at (`depth`, `index`) under `root`. As in the spec and in
/// Helios, a branch from a fork with a deeper tree may carry leading zero nodes.
fn is_valid_branch(leaf: Root, branch: &[B256], depth: usize, index: u64, root: &Root) -> bool {
    let Some(extra) = branch.len().checked_sub(depth) else { return false };
    if branch[..extra].iter().any(|n| !n.is_zero()) {
        return false;
    }
    let node = branch[extra..].iter().enumerate().fold(leaf, |node, (i, sib)| {
        if (index >> i) & 1 == 1 { sha256(&sib.0, &node) } else { sha256(&node, &sib.0) }
    });
    &node == root
}

/// `is_valid_light_client_header`: the header's execution commitment under its body root.
fn is_valid_header(header: &LightClientHeader, forks: &Forks) -> bool {
    let beacon = header.beacon();
    let epoch = beacon.slot / 32;
    match header {
        LightClientHeader::Gloas(h) => {
            is_valid_branch(h.execution_block_hash.0, &h.execution_branch, 11, 808, &beacon.body_root.0)
        }
        _ if epoch >= forks.gloas.epoch => false,
        _ => match (header.execution(), header.execution_branch()) {
            (Ok(e), Ok(branch)) => is_valid_branch(e.tree_hash_root().0, branch, 4, 9, &beacon.body_root.0),
            _ => false,
        },
    }
}

/// `Init`: the store and first record of an instance bound to `cfg`, from a bootstrap whose
/// header must hash to `checkpoint`, and the bootstrap committee's keys as SSZ.
pub fn bootstrap(cfg: &ChainConfig, checkpoint: &Root, bootstrap: &[u8], committee: &[u8], rlp: Option<&[u8]>) -> Result<(Store, HeaderRecord), Error> {
    let forks = cfg.forks()?;
    let bootstrap: Bootstrap<Spec> = decode(bootstrap)?;
    let header = bootstrap.header();
    let beacon = header.beacon();
    if beacon.tree_hash_root().0 != *checkpoint {
        return Err(Error::CheckpointMismatch);
    }
    if !is_valid_header(&header, &forks) {
        return Err(Error::BadExecutionBranch);
    }
    let root = committee_root_ssz(committee)?;
    let epoch = beacon.slot / 32;
    let (index, depth) = if epoch >= forks.gloas.epoch {
        (897, 11)
    } else if epoch >= forks.electra.epoch {
        (22, 6)
    } else {
        (22, 5)
    };
    if !is_valid_branch(root, bootstrap.current_sync_committee_branch(), depth, index, &beacon.state_root.0) {
        return Err(Error::BadCommitteeBranch);
    }
    let record = record_of(cfg, &header, rlp, 0)?;
    let store = Store { finalized_slot: beacon.slot, finalized_header: encode(&header), current_committee: root, next_committee: None };
    Ok((store, record))
}

/// The committee of SSZ `bytes`, checked against `root`.
fn committee(bytes: &[u8], root: &Root) -> Result<Committee, Error> {
    if committee_root_ssz(bytes)? != *root {
        return Err(Error::CommitteeMismatch);
    }
    Committee::from_ssz_bytes(bytes).map_err(|_| Error::Malformed)
}

/// `Update`: the store after `update` on top of `store`, and the record of the newly finalized
/// header, if there is one. `current` and `next` are the keys of the committees `store` holds
/// the roots of.
pub fn update(
    cfg: &ChainConfig,
    store: &Store,
    update: &[u8],
    current: &[u8],
    next: Option<&[u8]>,
    rlp: Option<&[u8]>,
) -> Result<(Store, Option<HeaderRecord>), Error> {
    let forks = cfg.forks()?;
    let update: AnyUpdate = decode(update)?;
    let signature_slot = update.signature_slot();
    if cfg.slot_time(signature_slot) > cfg.slot_time(store.finalized_slot) + cfg.max_store_age {
        return Err(Error::CheckpointStale);
    }
    let signers = update.signers();
    if signers * 3 < COMMITTEE * 2 {
        return Err(Error::BelowParticipation);
    }
    let next_committee = match (next, &store.next_committee) {
        (Some(bytes), Some(root)) => Some(committee(bytes, root)?),
        (None, None) => None,
        _ => return Err(Error::CommitteeMismatch),
    };
    let finalized: LightClientHeader = decode(&store.finalized_header)?;
    let mut helios = LightClientStore::<Spec> {
        finalized_header: finalized.clone(),
        optimistic_header: finalized,
        current_sync_committee: committee(current, &store.current_committee)?,
        next_sync_committee: next_committee,
        ..Default::default()
    };
    // No clock is read: the transaction's validity window bounds the time (see the program).
    let genesis = b256(&cfg.genesis_validators_root);
    match &update {
        AnyUpdate::Update(u) => verify_update(u, signature_slot, &helios, genesis, &forks),
        AnyUpdate::Finality(u) => verify_finality_update(u, signature_slot, &helios, genesis, &forks),
    }
    .map_err(|e| Error::from_helios(&e))?;
    let before = (helios.finalized_header.beacon().slot, helios.next_sync_committee.is_some());
    match &update {
        AnyUpdate::Update(u) => apply_update(&mut helios, u),
        AnyUpdate::Finality(u) => apply_finality_update(&mut helios, u),
    };
    let after = (helios.finalized_header.beacon().slot, helios.next_sync_committee.is_some());
    if after == before {
        return Err(Error::AlreadyApplied);
    }
    let header = helios.finalized_header;
    let record = if after.0 > before.0 { Some(record_of(cfg, &header, rlp, signers as u16)?) } else { None };
    // The roots after the update, without hashing a committee twice: a handoff moves the known
    // next root to current, and only a newly learned next committee is hashed.
    let rotated = lc_period(header.beacon().slot) > lc_period(store.finalized_slot);
    let current_committee = if rotated { store.next_committee.ok_or(Error::HandoffNotVerified)? } else { store.current_committee };
    let next_committee = match (&helios.next_sync_committee, rotated) {
        (None, _) => None,
        (Some(_), false) if store.next_committee.is_some() => store.next_committee,
        (Some(c), _) => Some(committee_root(c)),
    };
    let next_store = Store { finalized_slot: header.beacon().slot, finalized_header: encode(&header), current_committee, next_committee };
    Ok((next_store, record))
}

fn lc_period(slot: u64) -> u64 {
    slot / 8192
}

/// The CBOR the program decodes, for the client and tests.
pub fn cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {
    encode(value)
}
