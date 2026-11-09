//! The core against Helios: on recorded mainnet, from the checkpoint through two committee
//! handoffs to a finality update, both accept every update, in the same order, and reach the
//! same store; and the SSZ roots the core computes equal Helios's.
//!
//!   cargo test --release --test oracle

mod support;

use eth_lc_types::core::{self, beacon_root};
use eth_lc_types::oracle;
use support::{mainnet, steps};
use tree_hash::TreeHash;

#[test]
fn core_and_helios_walk_mainnet_to_the_same_store() {
    let chain = mainnet();
    let (steps, store) = steps(&chain);
    assert!(steps.len() >= 3, "every update in the segment applies");
    for (i, s) in steps.iter().enumerate() {
        let (after, advanced) = s.apply(&chain).unwrap();
        let h = s.helios(&chain, &s.update).expect("Helios accepts it too");
        assert_eq!(after.finalized.slot, h.finalized_header.beacon().slot, "step {i}: finalized slot");
        assert_eq!(beacon_root(&after.finalized), h.finalized_header.beacon().tree_hash_root().0, "step {i}: finalized header");
        assert_eq!(after.current_committee, h.current_sync_committee.tree_hash_root().0, "step {i}: current committee");
        assert_eq!(after.next_committee, h.next_sync_committee.as_ref().map(|c| c.tree_hash_root().0), "step {i}: next committee");
        println!("step {i}: slot {} -> {} (advanced {advanced}), same store as Helios", s.store.finalized.slot, after.finalized.slot);
    }
    assert_eq!(core::period(store.finalized.slot), 1906);
}

#[test]
fn ssz_roots_equal_helios() {
    let chain = mainnet();
    for c in &chain.committees {
        assert_eq!(core::committee_root(&oracle::committee(c)).unwrap(), c.tree_hash_root().0, "committee");
    }
    for u in &chain.updates {
        let (ours, theirs) = (oracle::update(u), match u {
            oracle::AnyUpdate::Update(u) => (u.attested_header(), u.finalized_header()),
            oracle::AnyUpdate::Finality(u) => (u.attested_header(), u.finalized_header()),
        });
        for (o, h) in [(&ours.attested, &theirs.0), (&ours.finalized, &theirs.1)] {
            assert_eq!(beacon_root(&o.beacon), h.beacon().tree_hash_root().0, "beacon header");
            if let core::Execution::Payload(p) = &o.execution {
                assert_eq!(core::payload_root(p), h.execution().unwrap().tree_hash_root().0, "payload header");
            }
        }
    }
}
