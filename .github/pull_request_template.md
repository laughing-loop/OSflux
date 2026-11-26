## What this changes

<!-- What does this pull request do, and why? Link the issue it fixes, for example "Fixes #12". -->

## How it was tested

<!-- The tests you added or changed, and anything you tried by hand. If you tested on a real drive, give the drive and the image. -->

## Checklist

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo test --workspace --all-features --locked` pass locally.
- [ ] If this touches drive writes or the safety chain (what is written, or whether OSflux may write), it adds tests for the new behaviour and for what must still be refused. See [CONTRIBUTING.md](https://github.com/laughing-loop/OSflux/blob/main/CONTRIBUTING.md#changes-that-touch-drive-writes-need-tests).
- [ ] Any new dependency has a GPL-3.0-compatible licence, and `Cargo.lock` is committed.
- [ ] `CHANGELOG.md` has a line under `[Unreleased]` for anything a user would notice.
- [ ] The README or `DOCUMENTS/` are updated if behaviour, commands or options changed.
- [ ] I agree that my contribution is licensed under the licence of the code it changes: GPL-3.0-or-later for the engine, CLI and core, GPL-3.0-only for osflux-studio.
