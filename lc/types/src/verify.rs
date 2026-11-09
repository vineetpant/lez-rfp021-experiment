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

use crate::error::Error;
use crate::{ChainConfig, HeaderRecord, Root, Store, b256};
use alloy_consensus::Header;
use alloy_rlp::Decodable;
use helios_consensus_core::consensus_spec::MainnetConsensusSpec as Spec;
use helios_consensus_core::types::{Bootstrap, FinalityUpdate, LightClientHeader, LightClientStore, SyncCommittee, Update};
use helios_consensus_core::{apply_bootstrap, apply_finality_update, apply_update, verify_bootstrap, verify_finality_update, verify_update};
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

/// `Init`: the store and first record of an instance bound to `cfg`, from a bootstrap whose
/// header must hash to `checkpoint`.
pub fn bootstrap(cfg: &ChainConfig, checkpoint: &Root, bootstrap: &[u8], rlp: Option<&[u8]>) -> Result<(Store, HeaderRecord), Error> {
    let forks = cfg.forks()?;
    let bootstrap: Bootstrap<Spec> = decode(bootstrap)?;
    verify_bootstrap(&bootstrap, b256(checkpoint), &forks).map_err(|e| Error::from_helios(&e))?;
    let mut helios = LightClientStore::default();
    apply_bootstrap(&mut helios, &bootstrap);
    let header = helios.finalized_header;
    let record = record_of(cfg, &header, rlp, 0)?;
    let store = Store {
        finalized_slot: header.beacon().slot,
        finalized_header: encode(&header),
        current_committee: bootstrap.current_sync_committee().tree_hash_root().0,
        next_committee: None,
    };
    Ok((store, record))
}

/// The committee of `bytes`, checked against `root`.
fn committee(bytes: &[u8], root: &Root) -> Result<Committee, Error> {
    let c: Committee = decode(bytes)?;
    if c.tree_hash_root() != b256(root) {
        return Err(Error::CommitteeMismatch);
    }
    Ok(c)
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
    let next_store = Store {
        finalized_slot: header.beacon().slot,
        finalized_header: encode(&header),
        current_committee: helios.current_sync_committee.tree_hash_root().0,
        next_committee: helios.next_sync_committee.as_ref().map(|c| c.tree_hash_root().0),
    };
    Ok((next_store, record))
}

/// The SSZ root of a committee, for the client and tests.
pub fn committee_root(c: &Committee) -> Root {
    c.tree_hash_root().0
}

/// The CBOR the program decodes, for the client and tests.
pub fn cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {
    encode(value)
}
