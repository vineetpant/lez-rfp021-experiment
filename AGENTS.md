# Notes for coding agents

`CLAUDE.md` is the full context for this repo: what it is, the decisions taken and why,
the traps that cost time, and the roadmap toward an RFP-021 implementation. **Read it
first.** This file exists so agents that look for `AGENTS.md` by convention find their way
there instead of rediscovering everything, and it is kept short on purpose — one copy of
the facts, in `CLAUDE.md`, so the two cannot drift apart.

In one paragraph: this is a verified-working hello world on LEZ (Logos Execution Zone),
standalone — no `logos-execution-zone` checkout needed. It exists to be the foundation for
a Logos RFP-021 grant proposal and implementation (trustless Ethereum state attestation).
`guest/` holds the on-chain program and is where the attestation module will live.

## Before you touch anything

- `./scripts/hello.sh` must still end in `PASS`. It asserts the greeting round-trips, so it
  is a real regression test, not a smoke test. Run it after any change to `src/`, `guest/`,
  `configs/` or `scripts/`.
- Scripts are **bash**. `scripts/common.sh` relies on `BASH_SOURCE`, which zsh does not set;
  sourcing it from zsh silently resolves every path one directory too high.
- Do not export `RISC0_DEV_MODE` globally. risc0's build scripts hash the environment, so it
  forces guest rebuilds on every cargo invocation. Use `DEV_MODE_ENV` from `common.sh`.
- Version pins live in two places by design: `LEZ_TAG` in `scripts/common.sh` and
  `BEDROCK_VERSION` in `docker-compose.yml`. Different repos, different cadences.
- The comments marking workarounds for upstream defects are load-bearing. Each says why it
  exists so it does not get tidied away as noise. `README.md` has the numbered table.

## House rules

- **Ask before installing toolchains, cloning repos, or downloading dependencies.** State
  what each step touches and wait. Read-only checks need no announcement. Prefer reversible
  moves over destructive ones.
- **Verify before claiming.** "Should work" is not a result. If something is untested, say
  so plainly rather than implying it passed.
- Comments explain why, not what.
