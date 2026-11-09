//! Shared by the native tests: the recorded mainnet segment, walked forward the way the program
//! advances an instance.

use eth_lc_types::verify::{self, AnyUpdate, cbor, committee_bytes};
use eth_lc_types::{HeaderRecord, Store};
use lez_hello_world::chain::{Chain, load_chain};

/// Periods 1904–1906 of mainnet: a bootstrap at slot 15597568, the best update of each period
/// (two committee handoffs), and a finality update. Fetched with `scripts/fetch-chain.sh`.
pub fn mainnet() -> Chain {
    load_chain(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mainnet-1904"), 1).expect("the recorded segment loads")
}

/// One step: the store before it, the update, and the committee keys it is checked with.
pub struct Step {
    pub store: Store,
    pub update: AnyUpdate,
    pub current: Vec<u8>,
    pub next: Option<Vec<u8>>,
}

impl Step {
    pub fn apply(&self, chain: &Chain) -> Result<(Store, Option<HeaderRecord>), eth_lc_types::error::Error> {
        verify::update(&chain.cfg, &self.store, &cbor(&self.update), &self.current, self.next.as_deref(), None)
    }
}

/// The keys for `store`'s committees, from the segment.
pub fn keys(chain: &Chain, store: &Store) -> (Vec<u8>, Option<Vec<u8>>) {
    let current = committee_bytes(chain.committee(&store.current_committee).expect("current committee in the segment"));
    let next = store.next_committee.map(|r| committee_bytes(chain.committee(&r).expect("next committee in the segment")));
    (current, next)
}

/// Every update that advances the store, in order, from the bootstrap; and the final store.
pub fn steps(chain: &Chain) -> (Vec<Step>, Store) {
    let (mut store, _) = verify::bootstrap(&chain.cfg, &chain.checkpoint, &cbor(&chain.bootstrap), &committee_bytes(chain.bootstrap.current_sync_committee()), None).expect("bootstrap");
    let mut out = vec![];
    for update in &chain.updates {
        let (current, next) = keys(chain, &store);
        let step = Step { store: store.clone(), update: clone(update), current, next };
        if let Ok((after, _)) = step.apply(chain) {
            store = after;
            out.push(step);
        }
    }
    (out, store)
}

pub fn clone(u: &AnyUpdate) -> AnyUpdate {
    serde_cbor::from_slice(&cbor(u)).unwrap()
}
