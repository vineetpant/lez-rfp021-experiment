//! The honest chain: Helios's checks, as the program wraps them, walk recorded mainnet forward
//! from a checkpoint through two committee handoffs to a finality update.
//!
//!   cargo test --release --test lc

mod support;

use support::{mainnet, steps};

#[test]
fn mainnet_verifies_from_the_checkpoint_through_two_handoffs() {
    let chain = mainnet();
    let (steps, store) = steps(&chain);
    println!("{} steps applied; finalized slot {}", steps.len(), store.finalized_slot);
    for (i, s) in steps.iter().enumerate() {
        let (after, record) = s.apply(&chain).unwrap();
        println!(
            "step {i}: slot {} -> {}, record {}",
            s.store.finalized_slot,
            after.finalized_slot,
            record.map_or("none".to_owned(), |r| format!("block {} signers {}", r.block_number, r.signers))
        );
    }
    // Two handoffs, then the finality update, which is two periods past the checkpoint.
    assert!(steps.len() >= 3, "every update in the segment applies");
    assert_eq!(store.finalized_slot / 8192, 1906);
}
