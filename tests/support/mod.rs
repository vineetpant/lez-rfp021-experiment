//! Shared by the native tests: the recorded mainnet segment, walked forward the way the program
//! advances an instance, with Helios walking it alongside as the oracle.
#![allow(dead_code)]

use eth_lc_types::core::{self, Store};
use eth_lc_types::error::Refusal;
use eth_lc_types::oracle::{self, AnyUpdate};
use helios_consensus_core::consensus_spec::MainnetConsensusSpec as Spec;
use helios_consensus_core::types::LightClientStore;
use lez_hello_world::chain::{Chain, load_chain};

/// Periods 1904–1906 of mainnet: a bootstrap at slot 15597568, the best update of each period
/// (two committee handoffs), and a finality update. Fetched with `scripts/fetch-chain.sh`.
pub fn mainnet() -> Chain {
    load_chain(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904"), 1).expect("the recorded segment loads")
}

/// One step: the store before it (the core's and Helios's), the update, and the committee keys
/// the core checks it with.
pub struct Step {
    pub store: Store,
    pub helios: LightClientStore<Spec>,
    pub update: AnyUpdate,
    pub current: Vec<u8>,
    pub next: Option<Vec<u8>>,
}

impl Step {
    pub fn apply(&self, chain: &Chain) -> Result<(Store, bool), Refusal> {
        self.apply_other(chain, &self.update)
    }

    /// The core's verdict on `update` in place of this step's.
    pub fn apply_other(&self, chain: &Chain, update: &AnyUpdate) -> Result<(Store, bool), Refusal> {
        core::update(&chain.cfg, &self.store, &oracle::update(update), &self.current, self.next.as_deref())
    }

    /// Helios's verdict on `update` on this step's store.
    pub fn helios(&self, chain: &Chain, update: &AnyUpdate) -> Result<LightClientStore<Spec>, String> {
        oracle::helios_update(&chain.cfg, &self.helios, update)
    }
}

/// The store a bootstrap gives, in the core and in Helios.
pub fn boot(chain: &Chain) -> (Store, LightClientStore<Spec>) {
    let b = oracle::bootstrap(&chain.bootstrap);
    let committee = chain.bootstrap.current_sync_committee();
    let store = core::bootstrap(&chain.cfg, &chain.checkpoint, &b, &oracle::committee(committee)).expect("bootstrap");
    (store, oracle::helios_store(&chain.bootstrap.header(), committee, None))
}

/// The keys for `store`'s committees, from the segment.
pub fn keys(chain: &Chain, store: &Store) -> (Vec<u8>, Option<Vec<u8>>) {
    let current = oracle::committee(chain.committee(&store.current_committee).expect("current committee in the segment"));
    let next = store.next_committee.map(|r| oracle::committee(chain.committee(&r).expect("next committee in the segment")));
    (current, next)
}

/// Every update that advances the store, in order, from the bootstrap; and the final store. The
/// core's store drives it; Helios's follows.
pub fn steps(chain: &Chain) -> (Vec<Step>, Store) {
    let (mut store, mut helios) = boot(chain);
    let mut out = vec![];
    for update in &chain.updates {
        let (current, next) = keys(chain, &store);
        let step = Step { store: store.clone(), helios: helios.clone(), update: update.clone(), current, next };
        if let Ok((after, _)) = step.apply(chain) {
            helios = step.helios(chain, update).expect("Helios applies what the core applies");
            store = after;
            out.push(step);
        }
    }
    (out, store)
}
