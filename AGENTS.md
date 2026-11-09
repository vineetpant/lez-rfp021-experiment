# Notes for coding agents

Read `CLAUDE.md` first. It has the full picture: what this repo is, why it's built the way it
is, what's known to be rough, and what's planned next. This file is deliberately short so the
facts only live in one place and the two can't drift apart.

The short version: this is a working hello world on LEZ (Logos Execution Zone). It runs
standalone — you don't need a copy of `logos-execution-zone`. It exists to become the base for
a Logos RFP-021 implementation, and `guest/` is where that program will live.

## Before you change anything

- **Ask the maintainer first.** Say what you want to change and why, then wait for approval. Reading
  code, answering questions and read-only checks are fine.
- **Commit messages are one line.** No body, no file lists. Never add AI attribution or
  co-author trailers.
- **Ask before installing or downloading anything**, and say what it touches.
- `./scripts/hello.sh` must still end in `PASS`. It checks the greeting actually came back, so
  it's a real test, not a smoke test. Run it after touching `src/`, `guest/`, `configs/` or
  `scripts/`.
- Scripts are bash, not zsh. `common.sh` uses `BASH_SOURCE`, which zsh doesn't set — source it
  from zsh and every path lands one directory too high.
- Don't export `RISC0_DEV_MODE` globally. It forces a guest rebuild on every cargo call. Use
  `DEV_MODE_ENV` from `common.sh`.
- The two version pins are separate on purpose: `LEZ_TAG` in `scripts/common.sh`,
  `BEDROCK_VERSION` in `docker-compose.yml`.
- Comments that explain why something is set a certain way are doing real work. Several of them
  guard against behaviour you'd never guess. Don't tidy them away.
