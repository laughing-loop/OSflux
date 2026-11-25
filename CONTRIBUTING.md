# Contributing to OSflux

Thank you for helping. OSflux writes to drives and erases them, so this guide asks a little more care than most projects do, especially around drive writes. Everything else is the usual: open an issue or a pull request, keep changes focused, and make CI pass.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). To report a security problem, do not open an issue: follow [SECURITY.md](SECURITY.md).

## Where to start

- Look for issues labelled `good first issue`.
- For anything bigger than a small fix, open an issue first and say what you plan to do, so nobody duplicates work and the approach can be agreed before you write it.
- [DOCUMENTS/PROJECT_OVERVIEW.md](DOCUMENTS/PROJECT_OVERVIEW.md) explains the project in plain terms, [DOCUMENTS/ARCHITECTURE.md](DOCUMENTS/ARCHITECTURE.md) and [DOCUMENTS/WRITE_ENGINE.md](DOCUMENTS/WRITE_ENGINE.md) explain how it is built, and [DOCUMENTS/RELEASING.md](DOCUMENTS/RELEASING.md) covers CI and releases.

The workspace has four crates:

| Crate | What it holds |
|---|---|
| `crates/osflux-core` | Device access, Windows I/O, privilege checks and the low-level write pipeline |
| `crates/osflux-engine` | Image planning, the safety chain, confirmations, the audit log, writing and verifying |
| `crates/osflux-cli` | The `osflux` program |
| `crates/osflux-studio` | OSflux Studio, the desktop app, built with Slint |

## Setting up

Rust is installed through [rustup](https://rustup.rs). The version is pinned in `rust-toolchain.toml`, and rustup installs it, with `rustfmt` and `clippy`, the first time you run `cargo` in the repository. You do not need to pick a version yourself.

### Windows

- rustup, with the default `x86_64-pc-windows-msvc` toolchain
- Visual Studio Build Tools with the "Desktop development with C++" workload, which includes the MSVC compiler and the Windows SDK

### Linux

On Debian or Ubuntu:

```sh
sudo apt install build-essential libfontconfig-dev
```

`libfontconfig-dev` is needed to build Studio. These tools are optional, but without them some tests skip their cross-checks against independent implementations, which CI always runs:

```sh
sudo apt install mtools dosfstools wimtools libxml2-utils
```

To run the Windows build check that CI runs on Linux, also install the MinGW toolchain and target:

```sh
sudo apt install gcc-mingw-w64-x86-64
rustup target add x86_64-pc-windows-gnu
```

OSflux builds and its tests run on Linux, but listing and writing real drives is only implemented on Windows so far.

## Building, testing and running

The commands are the same on Windows and Linux.

```sh
cargo build                     # the osflux CLI (the default member)
cargo build --workspace         # every crate, including Studio
cargo test --workspace --all-features
cargo run -- --help             # the CLI
cargo run -p osflux-studio      # Studio
```

Studio has a preview mode that shows any screen with sample drives, so nothing is read or written:

```sh
cargo run -p osflux-studio --features preview -- --preview gallery --theme dark
```

The README lists every CLI command and Studio view.

**Never point a write at a drive you care about.** Commands that only plan, assess or print, such as `--plan-image`, `--assess-target`, `--preflight` and `--intent`, never write. Only `--write ... --execute` writes, and it erases the target. Use a spare USB stick for manual testing.

## The checks every pull request must pass

CI (`.github/workflows/ci.yml`) runs on every pull request to `main`. All of these must pass before a pull request can be merged:

| Check | Runs on | Command |
|---|---|---|
| Formatting | Linux | `cargo fmt --all -- --check` |
| Lints, with warnings as errors | Windows and Linux | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| Tests | Windows and Linux | `cargo test --workspace --all-features --locked` |
| Release build of `osflux.exe` and `osflux-studio.exe` | Windows | `cargo build --release --locked -p osflux-cli -p osflux-studio` |
| Windows build check from Linux | Linux | `cargo check --workspace --all-targets --all-features --locked --target x86_64-pc-windows-gnu` |
| Dependency licences | Linux | `cargo deny --locked check licenses` |

Run the first three locally before you push:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

`--locked` fails if `Cargo.lock` is out of date. If you changed dependencies, run a build without it first so `Cargo.lock` is updated, and commit `Cargo.lock` with your change.

### Adding a dependency

Every dependency, including indirect ones, must use a licence compatible with GPL-3.0. The licence check fails on anything outside the allow-list in `deny.toml`. Run it locally with:

```sh
cargo install cargo-deny --locked
cargo deny check licenses
```

If a new crate's licence is not on the list, check it against the [FSF's list of GPL-compatible licences](https://www.gnu.org/licenses/license-list.html) and explain it in the pull request. A crate under an incompatible licence, such as GPL-2.0-only, cannot be used. Prefer few, well-maintained dependencies.

## Changes that touch drive writes need tests

A bug in OSflux can erase the wrong drive or leave a stick that does not boot, so any change that touches how OSflux writes to a drive, or decides whether it may, must come with tests that cover it. That includes:

- the write path: `crates/osflux-core/src/writer/`, `crates/osflux-core/src/device/`, `crates/osflux-core/src/windows_io.rs`, `crates/osflux-engine/src/raw_write.rs`, and anything that builds what is written, such as `fat32.rs`, `file_copy.rs`, `wim_split.rs` and `answer_file.rs`;
- the safety chain and the decision to write: target assessment, drive contents, preflight, write intent and authorization, confirmations, the session, and the audit log;
- the CLI and Studio code that starts a write or passes options to it.

The tests must show the new behaviour and, where it applies, that OSflux refuses what it should refuse: the wrong drive, a changed drive, a stale or reused confirmation, an image that does not fit. A pull request that changes this code without tests will not be merged, however small the change.

Tests never touch a real drive. The engine's unit tests use in-memory drives and the sparse drive images under `crates/osflux-engine/tests/fixtures/`, which the `make-fixtures.sh` scripts there rebuild with the standard Linux tools. Where an independent tool can check OSflux's output, such as `mtools` and `fsck.fat` for FAT32 volumes, `wimlib` for split WIMs and `xmllint` for answer files, add that check too.

A hardware test on a real USB stick, `scripts/test-drive-contents.ps1`, is run by maintainers before a release. You are welcome to run it, but it erases the stick it selects. If you tested a change on real hardware, say so in the pull request, with the drive and image you used.

## Pull requests

- Branch from `main`, and keep each pull request to one change. Unrelated fixes go in their own pull requests.
- Fill in the pull request template.
- Add a line to the `## [Unreleased]` section of `CHANGELOG.md` for anything a user would notice.
- Update the README or `DOCUMENTS/` when behaviour, commands or options change.
- Pull requests are merged by a maintainer once CI passes and the change has been reviewed.

## Commit style

Look at `git log` for examples. In short:

- Write the subject as a plain-English sentence in the imperative, saying what the change does for the user or the code: `Block images larger than the target drive`, `Ask before forcing a busy drive free, the way chkdsk and format do`.
- Start with a capital letter, put no full stop at the end, and keep it under about 72 characters. Changes to Studio may start with `Studio: `.
- Do not use prefixes such as `feat:` or `fix:`.
- If the reason is not obvious, add a body after a blank line, wrapped at about 72 characters, explaining why.

## Licence

OSflux is licensed under the GNU General Public License, version 3 or (at your option) any later version (`GPL-3.0-or-later`). See [LICENSE](LICENSE). OSflux Studio as a whole is distributed under GPL-3.0 only, because it includes Slint, which is licensed that way.

By submitting a contribution, you agree that it is licensed under the same terms, and that you have the right to submit it under them. Do not include code copied from projects under incompatible licences.

The licence does not cover the OSflux name or logo, including the files in `assets/brand/`; see the Licence section of the README.
