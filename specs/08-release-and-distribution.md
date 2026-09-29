# Release and Distribution Specification

This describes the existing `.github/workflows/release.yml` pipeline and the npm multi-package layout it publishes into — not a proposal, a description of what must keep working.

## Versioning

- `Cargo.toml`'s `version` field is the single source of truth for the release version. The git tag for a release is `v<version>`.
- On a push to `main`, the `check-version` job reads `Cargo.toml`, checks whether tag `v<version>` already exists, and only proceeds with a release if it doesn't (i.e. a version bump in `Cargo.toml` is what triggers a release, not every push).
- `workflow_dispatch` allows an explicit version input instead (for manual/test releases), bypassing the tag-existence check.
- Every npm package under `npm/*/package.json` (the wrapper and all six platform packages) has its `version` field overwritten to match the release version during the `publish-npm` job. Do not hand-maintain these version fields for correctness — they're set by CI on every real release; a stale value in git between releases is expected and harmless.

## Build matrix

| Target triple | Runner | Toolchain | Artifact |
|---|---|---|---|
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | `cargo build`, `mold` linker | `ime-linux-amd64` |
| `aarch64-unknown-linux-musl` | `ubuntu-latest` | `cargo-zigbuild` | `ime-linux-arm64` |
| `armv7-unknown-linux-musleabihf` | `ubuntu-latest` | `cargo-zigbuild` | `ime-linux-armv7` |
| `aarch64-apple-darwin` | `macos-latest` | native `cargo build` | `ime-macos-arm64` |
| `x86_64-apple-darwin` | `macos-latest` | native `cargo build` | `ime-macos-amd64` |
| `x86_64-pc-windows-msvc` | `ubuntu-latest` | `cargo-xwin` (cross) | `ime-windows-amd64.exe` |

All Linux targets are `musl` (static libc), not `gnu`, so the binaries run on any Linux distribution regardless of installed glibc version. Every dependency in `Cargo.toml` must keep building under all six of these; in particular, `arboard`'s Linux clipboard backend (`x11rb`) talks to the X server over a raw socket by default and does not need `libxcb.so`/`libX11.so`, so it's safe under static `musl` linking without extra system packages. A minority of environments (a Wayland compositor with no XWayland) may still lack clipboard support, since the `wayland-data-control` feature is not enabled — acceptable, not a blocker.

Each `workflow_dispatch` run can selectively enable/disable the Linux, Linux-ARM, macOS, and Windows build jobs; a plain push to `main` always builds all of them.

## Publishing gates

- A normal push-to-`main` release, once a new version is detected, always publishes to both crates.io and npm after all four build jobs succeed.
- A `workflow_dispatch` run only publishes if its `full_publish` input is explicitly set to `true`; otherwise it builds and creates a GitHub Release without publishing to either registry (a dry-run/rehearsal mode).
- `publish-crate` runs `cargo publish` once, from the workspace root.
- `publish-npm` publishes up to seven packages: one per platform (only if that platform's binary artifact was actually produced — missing artifacts are skipped via `continue-on-error`/file-existence checks, not treated as failures) plus the wrapper package last.

## npm package layout

- `@avaray/ime` (in `npm/ime-cli/`) is the package people actually install. It has no compiled code of its own: `bin.js` is a thin Node.js dispatcher with no third-party dependencies.
- Six platform-specific packages (`@avaray/ime-linux-x64`, `@avaray/ime-linux-arm64`, `@avaray/ime-linux-armv7`, `@avaray/ime-macos-x64`, `@avaray/ime-macos-arm64`, `@avaray/ime-win32-x64`) each contain nothing but the single native `ime`/`ime.exe` binary. They are declared as `optionalDependencies` on `@avaray/ime`, so `npm install` only downloads the one matching the installing machine.
- At runtime, `bin.js` maps `os.platform()`/`os.arch()` to the matching platform package name and `spawnSync`s that binary, forwarding argv and stdio. **The mapping is not a direct string interpolation** — Node's platform/arch strings don't match the npm package segments one-to-one:

  | `os.platform()` | package segment | `os.arch()` | package segment |
  |---|---|---|---|
  | `darwin` | `macos` | `x64` | `x64` |
  | `linux` | `linux` | `arm64` | `arm64` |
  | `win32` | `win32` | `arm` | `armv7` |

  `bin.js` must go through this explicit map, never `` `@avaray/ime-${os.platform()}-${os.arch()}` `` directly — that produces `ime-darwin-*` and `ime-linux-arm`, which don't exist as packages, silently breaking macOS and 32-bit ARM Linux while Linux x64/arm64 and Windows x64 happen to work by coincidence.
- If either lookup misses (an unsupported platform/arch combination, or the matching optional dependency failed to install), `bin.js` exits `1` with a message naming the expected package and the manual install command, rather than crashing on an unhandled exception.

## GitHub Release

- One release per version tag, with every produced binary attached as a raw file, a SHA-256 checksum table for all attached files, and a changelog section auto-generated from Conventional Commit messages (`feat`/`fix`/`perf`/`refactor`/`revert`) since the previous `vX.Y.Z` tag.
- The release step runs even if `full_publish` is off — a manual rehearsal still produces a real GitHub Release with real binaries, it just skips crates.io/npm.
