//! Attesting to a block older than the record it is proven from (proposal section 8).
//!
//! A record anchors one finalized block. [`target_record`] derives, from it, the execution roots
//! of an earlier block, so an attestation can be proven against that block without the instance
//! having recorded it:
//!
//! - **EIP-2935.** The history contract keeps the hashes of the 8,191 blocks before each block in
//!   its state. A storage proof under the anchor's `state_root` gives the target's hash, and the
//!   target's RLP header, which must hash to it, gives its roots.
//! - **A parent-hash chain.** The RLP headers from the anchor back to the target, each hashing to
//!   its child's `parentHash`, starting from the anchor's own block hash. It depends on nothing
//!   but header hashes, so it covers any distance and any chain, at a cost that grows with it.

use crate::error::Error;
use crate::mpt::{StorageProof, verify_storage};
use crate::HeaderRecord;
use alloy_consensus::Header;
use alloy_primitives::keccak256;
use alloy_rlp::Decodable;
use borsh::{BorshDeserialize, BorshSerialize};

/// The EIP-2935 history contract, at the same address on every chain that runs it.
pub const HISTORY_ADDRESS: [u8; 20] = alloy_primitives::hex!("0000F90827F1C53a10cb7A02335B175320002935");

/// How many block hashes the history contract keeps (`HISTORY_SERVE_WINDOW`).
pub const HISTORY_WINDOW: u64 = 8191;

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Ancestry {
    /// The history contract's slot for the target, proven under the anchor's state root, and
    /// the target's RLP header.
    Eip2935 { proof: StorageProof, header: Vec<u8> },
    /// RLP headers from the anchor's block back to the target, newest first.
    ParentChain { headers: Vec<Vec<u8>> },
}

/// The history contract's storage slot for block `number`.
pub fn history_slot(number: u64) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[24..].copy_from_slice(&(number % HISTORY_WINDOW).to_be_bytes());
    slot
}

fn header(rlp: &[u8]) -> Result<Header, Error> {
    Header::decode(&mut &rlp[..]).map_err(|_| Error::BadExecutionBranch)
}

/// The record of an earlier block, derived from `anchor` through `ancestry`. It carries the
/// target's execution fields; its beacon fields are zero, because nothing here proves them.
pub fn target_record(anchor: &HeaderRecord, ancestry: &Ancestry) -> Result<HeaderRecord, Error> {
    let (rlp, hash) = match ancestry {
        Ancestry::Eip2935 { proof, header: rlp } => {
            if proof.address != HISTORY_ADDRESS {
                return Err(Error::BadInclusion);
            }
            verify_storage(&anchor.state_root, proof)?;
            let number = header(rlp)?.number;
            // The slot is the target's, and the target is inside the window the anchor's state
            // keeps, so the hash in that slot is the target's and not a later block's.
            if proof.slot != history_slot(number) || number >= anchor.block_number || anchor.block_number - number > HISTORY_WINDOW {
                return Err(Error::BadInclusion);
            }
            (rlp, proof.value)
        }
        Ancestry::ParentChain { headers } => {
            let (first, rest) = headers.split_first().ok_or(Error::Malformed)?;
            if rest.is_empty() || keccak256(first).0 != anchor.block_hash {
                return Err(Error::BadExecutionBranch);
            }
            let mut child = first;
            for h in rest {
                if keccak256(h) != header(child)?.parent_hash {
                    return Err(Error::BadExecutionBranch);
                }
                child = h;
            }
            (child, keccak256(child).0)
        }
    };
    if keccak256(rlp).0 != hash {
        return Err(Error::BadExecutionBranch);
    }
    let target = header(rlp)?;
    Ok(HeaderRecord {
        chain_id: anchor.chain_id,
        slot: 0,
        beacon_root: [0; 32],
        block_number: target.number,
        block_hash: hash,
        state_root: target.state_root.0,
        receipts_root: target.receipts_root.0,
        signers: anchor.signers,
    })
}
