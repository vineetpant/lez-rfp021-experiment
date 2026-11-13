# Caisson: our RFP-021 proposal

The proposal is **Caisson**, Trustless Ethereum State Attestation for LEZ:
<https://claude.ai/artifact/NEstvDGmVwQC6zGXgWzpNS>

This repo is its working prototype. Where each part of the proposal lives here:

| Proposal | Here |
|---|---|
| 1. Design overview: the operator proves updates, LEZ checks the proof | `program/src/bin/eth_lc.rs` (`Update`, `Advance`, `WriteRecord`) |
| 3. Advancing finality: the sync-committee check | `types/src/core.rs` (`core::update`), held to Helios in `tests/oracle.rs`, `tests/adversarial.rs` |
| 4. Core data structures | `types/src/lib.rs` (`InstanceConfig`, `FinalisedHeader`, `Event`), `types/src/statement.rs` (`Claim`, `Fact`, `Condition`, `Statement`, `ProvenFact`). Not yet: the per-update `ConsensusState` and per-period `Committee` accounts (#40) |
| 5. Attestations, public and private | `Attest`, `AttestPrivate`, `CheckRecord`; measured in the README's "What it costs" |
| 6. Errors | `types/src/error.rs` (`Refusal`) |
| 7.1 History through EIP-2935, with a parent-hash fallback | `types/src/history.rs`, `tests/history.rs` |
| 10. Security model: fabricated chains and the other VS.2 cases | `tests/adversarial.rs`, `tests/statement.rs` |
| 12. Performance summary | the README's "What it costs", reproduced by `tests/cost.rs` and `scripts/run.sh` |

The design of claims, statement IDs and the consumer's nullifiers, with a worked 10 ETH deposit,
is in [statements.md](statements.md).
