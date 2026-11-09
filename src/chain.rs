//! A chain segment for `eth_lc`, read from Beacon API JSON (`scripts/fetch-chain.sh` writes it):
//! `genesis.json`, `fork_schedule.json`, one `bootstrap-<slot>.json` with its `.root`,
//! `updates-*.json` arrays, and optionally `finality_update.json`. Every object decodes straight
//! into Helios's types.

use std::path::Path;

use anyhow::{Context, anyhow};
use eth_lc_types::mpt::{Account, StorageProof};
use eth_lc_types::verify::{AnyUpdate, Committee, committee_root};
use eth_lc_types::{ChainConfig, Root};
use helios_consensus_core::consensus_spec::MainnetConsensusSpec as Spec;
use helios_consensus_core::types::{Bootstrap, FinalityUpdate, Forks, Update};
use serde_json::{Value, json};

/// Two weeks: inside Ethereum's weak-subjectivity period at today's validator count.
pub const MAX_STORE_AGE: u64 = 14 * 24 * 3600;

const FORK_NAMES: [&str; 8] = ["genesis", "altair", "bellatrix", "capella", "deneb", "electra", "fulu", "gloas"];

pub struct Chain {
    pub cfg: ChainConfig,
    pub checkpoint: Root,
    pub bootstrap: Bootstrap<Spec>,
    /// Every update in the segment: the periods' best updates in order, then the finality update.
    pub updates: Vec<AnyUpdate>,
    /// Every committee whose keys the segment carries: the bootstrap's, then each handoff's.
    pub committees: Vec<Committee>,
}

impl Chain {
    pub fn committee(&self, root: &Root) -> Option<&Committee> {
        self.committees.iter().find(|c| &committee_root(c) == root)
    }
}

fn read(path: &Path) -> anyhow::Result<Value> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

fn root(s: &str) -> anyhow::Result<Root> {
    let v = hex_decode(s)?;
    v.try_into().map_err(|_| anyhow!("{s} is not 32 bytes"))
}

pub fn hex_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let s = s.trim().trim_start_matches("0x");
    let s = if s.len() % 2 == 1 { format!("0{s}") } else { s.to_owned() };
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into)).collect()
}

/// Helios's `Forks` from the Beacon API's fork schedule, in fork order. A fork not yet
/// scheduled (Gloas on mainnet today) stays inactive.
fn forks(schedule: &Value) -> anyhow::Result<Forks> {
    let entries = schedule["data"].as_array().ok_or_else(|| anyhow!("no fork schedule"))?;
    let mut forks = serde_json::Map::new();
    for (name, e) in FORK_NAMES.iter().zip(entries) {
        let epoch: u64 = e["epoch"].as_str().unwrap_or("0").parse()?;
        forks.insert((*name).into(), json!({ "epoch": epoch, "fork_version": e["current_version"] }));
    }
    if !forks.contains_key("gloas") {
        forks.insert("gloas".into(), json!({ "epoch": u64::MAX, "fork_version": "0x00000000" }));
    }
    Ok(serde_json::from_value(Value::Object(forks))?)
}

pub fn load_chain(dir: impl AsRef<Path>, chain_id: u64) -> anyhow::Result<Chain> {
    let dir = dir.as_ref();
    let genesis = read(&dir.join("genesis.json"))?;
    let forks = forks(&read(&dir.join("fork_schedule.json"))?)?;
    let cfg = ChainConfig {
        chain_id,
        genesis_validators_root: root(genesis["data"]["genesis_validators_root"].as_str().unwrap_or_default())?,
        genesis_time: genesis["data"]["genesis_time"].as_str().unwrap_or("0").parse()?,
        forks: serde_cbor::to_vec(&forks)?,
        max_store_age: MAX_STORE_AGE,
    };

    let mut names: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    names.sort();
    let boot = names.iter().find(|n| n.starts_with("bootstrap-") && n.ends_with(".json")).ok_or_else(|| anyhow!("no bootstrap in {}", dir.display()))?;
    let bootstrap: Bootstrap<Spec> = serde_json::from_value(read(&dir.join(boot))?["data"].clone()).context("decode the bootstrap")?;
    let checkpoint = root(&std::fs::read_to_string(dir.join(boot.replace(".json", ".root")))?)?;

    let mut committees = vec![bootstrap.current_sync_committee().clone()];
    let mut updates = vec![];
    for n in names.iter().filter(|n| n.starts_with("updates-")) {
        for u in read(&dir.join(n))?.as_array().ok_or_else(|| anyhow!("{n} is not an array"))? {
            let u: Update<Spec> = serde_json::from_value(u["data"].clone()).with_context(|| format!("decode an update in {n}"))?;
            if *u.next_sync_committee() != Committee::default() {
                committees.push(u.next_sync_committee().clone());
            }
            updates.push(AnyUpdate::Update(u));
        }
    }
    if names.iter().any(|n| n == "finality_update.json") {
        let f: FinalityUpdate<Spec> = serde_json::from_value(read(&dir.join("finality_update.json"))?["data"].clone()).context("decode the finality update")?;
        updates.push(AnyUpdate::Finality(f));
    }
    Ok(Chain { cfg, checkpoint, bootstrap, updates, committees })
}

pub fn hex32(v: &serde_json::Value) -> anyhow::Result<[u8; 32]> {
    let b = hex_decode(v.as_str().unwrap_or_default())?;
    anyhow::ensure!(b.len() <= 32, "{v} is longer than 32 bytes");
    let mut w = [0u8; 32];
    w[32 - b.len()..].copy_from_slice(&b);
    Ok(w)
}

/// An `eth_getProof` response as a proof of one account and slot. Nodes shorten quantities
/// (`0x4a8`), so every number is left-padded back to its width.
pub fn storage_proof(path: &str) -> anyhow::Result<StorageProof> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let r = &j["result"];
    let nodes = |v: &serde_json::Value| -> anyhow::Result<Vec<Vec<u8>>> {
        v.as_array().ok_or_else(|| anyhow::anyhow!("{path}: not a proof"))?.iter().map(|x| hex_decode(x.as_str().unwrap_or_default())).collect()
    };
    let s = &r["storageProof"][0];
    let address = hex_decode(r["address"].as_str().unwrap_or_default())?;
    Ok(StorageProof {
        address: address.try_into().map_err(|_| anyhow::anyhow!("{path}: bad address"))?,
        account: Account {
            nonce: u64::from_str_radix(r["nonce"].as_str().unwrap_or("0x0").trim_start_matches("0x"), 16)?,
            balance: hex32(&r["balance"])?,
            storage_root: hex32(&r["storageHash"])?,
            code_hash: hex32(&r["codeHash"])?,
        },
        account_proof: nodes(&r["accountProof"])?,
        slot: hex32(&s["key"])?,
        value: hex32(&s["value"])?,
        storage_proof: nodes(&s["proof"])?,
    })
}
