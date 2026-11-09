//! The honest chain: the core walks recorded mainnet forward from a checkpoint through two
//! committee handoffs to a finality update.
//!
//!   cargo test --release --test lc

mod support;

use eth_lc_types::core;
use support::{mainnet, steps};

#[test]
fn mainnet_verifies_from_the_checkpoint_through_two_handoffs() {
    let chain = mainnet();
    let (steps, store) = steps(&chain);
    for (i, s) in steps.iter().enumerate() {
        let (after, advanced) = s.apply(&chain).unwrap();
        println!("step {i}: slot {} -> {} (advanced {advanced})", s.store.finalized.slot, after.finalized.slot);
    }
    assert!(steps.len() >= 3, "every update in the segment applies");
    assert_eq!(core::period(store.finalized.slot), 1906);
}
