# steadcopy · 稳拷

**Insert a card, it backs itself up, verifies both ends, and hands you a report in plain language.**

A Windows card-offload (DIT) tool for creators and small teams. Open source, MIT, **free forever including commercial use**.

> 📖 **[Getting started guide](https://hocassian.feishu.cn/docx/BAALdIhzvoKkPLxlr8icZ4Nwn6d)** — written for someone who has never used an offload tool (Chinese; Feishu translates in place)
> 中文版：[README.md](README.md)

---

## Why another one

The free tier does not lack checksum verification, and it does not lack correct architecture — what it lacks is **all of it in one shell built for creators, actually free**. Every existing option is missing a leg:

| Option | What's missing |
|---|---|
| DaVinci Resolve Clone Tool | Architecturally sound, but buried inside a multi-GB NLE and DIT jargon; the verify pass is slow enough that the official forum has complained for years; writes MHL but **cannot re-verify against it** |
| TeraCopy | Strong verification, but **multi-destination and report export are exactly what's behind Pro**, and the free tier excludes commercial use |
| FastCopy | Fast engine, single destination, free tier excludes commercial use |
| FreeFileSync | Sync-shaped, not offload-shaped; the checksum switch hides in a config file and the author admits the implementation is questionable |
| rclone / robocopy | Wrong audience; robocopy has no content verification at all |
| Hedge → OffShoot | **Dropped its free tier** when it renamed in 2023, and nothing has filled the gap since |

steadcopy fills that gap. It carries no revenue expectations, which is why it can be: **no account, no paywall, no server dependency, works offline, zero telemetry.**

## What it does today

- **Whole-card mirror by default.** Only system junk is excluded; files it doesn't recognise are copied anyway — one missing sidecar can break an entire clip in an NLE (Canon's `INDEX.MIF`, Insta360's paired `.insv`)
- **Read the source once, write every destination in parallel** (1–4). The source is read exactly once and fed to all destinations plus the hasher simultaneously
- **Hash while reading** (XXH64, MD5 optional) — no extra pass over the source
- **Read-back verification with caching bypassed** (`FILE_FLAG_NO_BUFFERING`). A "verification" that doesn't bypass the page cache may be reading a copy in RAM — that's worse than not verifying, because it looks like it worked
- **Automatic re-copy on verification failure**, with the retry set narrowing each round and exponential backoff
- **Resume.** The ledger is scoped per destination × card. "Already done" requires three things at once: recorded in the manifest, verified at the time, and the file still present at the destination
- **Four-state re-verification**: intact / moved / missing / added. Never collapsed into a boolean — what you want to know is **which** files have **what** problem
- **Reports in plain language**: single-file HTML, styles inlined, opens offline, prints to PDF, viewable inside the app
- **Credentials travel with the data**: the manifest lands inside the destination directory, so moving the folder moves the proof
- **The source card is read-only**: nothing is ever written to it, not even a device marker
- **Presets + insert-and-run.** Plug a card in and it knows which project and which parameters. Matching runs narrowest-first across three tiers (specific device / kind of device / any identified source); order is priority
- **Copying never requires a preset.** "Copy just once" is always available — presets are an accelerator, not a gate
- **Remember what you just did.** While a copy runs, an inline prompt offers to turn this run into a standing preset. Defaults to the narrowest scope (just this card)
- **Copy map.** Draw the destination folder tree as a node graph, wire each card to a folder node, and "Start all" lands every card exactly where the map says — each wire runs through the same execution and verification path as every other copy, and trees can be saved as templates. The concept honours a discontinued predecessor; this implementation is written from scratch and open source
- **Plugging in a card always produces a conclusion.** Nine distinct outcomes are surfaced, each with a next step — never "I plugged it in and nothing happened"
- **Unidentified devices never auto-run.** A device you've never told it about always stops at the identify step, and the danger-zone "skip confirmation" switch does not override that. Copying is recoverable; acting on an unknown device is not
- **Pause / resume / cancel**, responding within one chunk; cancelling wakes a paused task immediately
- **Safe eject**: lock → dismount → eject through system interfaces, no external helper binaries
- **Task ledger**: SQLite history, per-file detail, and a record of every format attempt. Schema is versioned and never silently rebuilt
- **Formatting lives in the danger zone**: off by default, double confirmation (type a phrase + countdown, minimum 10s), safety chain G1–G4, and every attempt is recorded whether it succeeded, failed, was rejected, or was cancelled

## Download and verify

Two installers, pick one:

| | Size | When to use it |
|---|---|---|
| `steadcopy_<version>_x64-setup.exe` | ~4 MB | **The default.** Installs straight away if WebView2 is already present; otherwise it fetches the runtime during install |
| `steadcopy_<version>_x64-setup-offline.exe` | ~206 MB | **No network on set, or a brand-new machine.** The WebView2 runtime is bundled in full, so it installs offline |

They are **two ways of installing the same product** — identical once installed, and either one upgrades the other in place. A portable zip (~7 MB) is also available: unzip and run, with all data kept next to the executable.

Where to get it:

- [GitHub Releases](https://github.com/Gitruck/steadcopy/releases)
- Mirror (better reachable from mainland China): `https://api.ai-mcn.tv:9000/broadcast/steadcopy/` — the same bytes as the Releases assets, same checksums

> **This build is not code-signed**, so Windows will warn about an unknown publisher on first run. That is expected.
> Verify the checksum below to confirm where the file came from. **Never turn off your security software in order to run this program.**

```powershell
Get-FileHash .\steadcopy_0.1.1_x64-setup.exe -Algorithm SHA256
```

Compare against the value published on the Releases page. Install only if they match.

**Two installers, pick one**: the slim build is ~4 MB (it assumes WebView2 is already present, which it is on Windows 11 and Windows 10 22H2), the offline build is ~206 MB (the runtime is bundled in full, so it installs with no network at all). No network at the shoot is a hard constraint, so the offline build has to exist — but making everyone download 206 MB for it would be unreasonable. A portable zip (~7 MB) is also provided.

## Command line

The engine and the GUI share one core, and the CLI is a first-class citizen (it's also the end-to-end test driver):

```bash
steadcopy devices                                    # list volumes, marking which can act as a source
steadcopy scan E:\ --list                            # scan a source
steadcopy plan E:\ -d D:\media -d F:\backup -p Wedding   # dry run, zero side effects
steadcopy copy E:\ -d D:\media -d F:\backup -p Wedding   # run it
steadcopy audit <manifest.json>                      # re-verify, four-state result
steadcopy watch                                      # sit and wait; run presets on insert
steadcopy eject E:                                   # safe eject
steadcopy format E: --yes-i-know-this-erases-data    # ⚠️ the dangerous flag is deliberately long
```

Every command supports `--json` (pure JSON on stdout, logs on stderr) and `--lang zh|en`. Exit codes: `0` success / `1` terminal / `2` retryable / `3` cancelled / `4` usage error. "Nothing new to copy" is a normal result and exits `0`.

## Language

Chinese and English. Follows the system language by default; switchable under Settings → Language, effective immediately.

> **Honest note about the current state:** the interface, the insert-and-run flow, the orchestration conclusions, the copy report, the CLI output, the safety-check details and all nine error families are **fully bilingual**.
> The one thing that stays Chinese is the CLI's `--help`: clap fixes it at compile time and it cannot switch at runtime. `--lang en` governs runtime output, and `--help` says so itself.
> A missing translation falls back to **Chinese, never to blank**: exhaustive `match` on the Rust side, `Record<Key, string>` on the TS side — miss one and it fails to compile.

## Build from source

```bash
cargo test                          # engine and CLI (safe track, zero side effects)
python scripts/check-scenarios.py   # spec scenario ↔ test coverage self-check
python scripts/build-release.py     # every release artifact in one command
```

`build-release.py` runs, in order: license gate (a GPL-family dependency stops it right there) → safe-track tests → static checks → CLI → installer → portable zip → checksums. Artifacts land in `release/`.

Requires Rust 1.85+, Bun, Python 3, and the Windows MSVC build tools.

## Known boundaries

- **Windows only.** The core is platform-agnostic and the macOS seams are left in place, but it hasn't been built.
- **Update checks are off by default and never auto-install.** No account, no telemetry, no background polling — even when enabled, it goes online only when you press "Check for updates". Finding a new version just tells you; installing is your call. Update packages are signed with the release key and verified offline against a public key compiled into the app.
- **Phones over MTP are not a trustworthy backup path.** Android and iPhone appear as "portable devices" on Windows: no drive letter, not filesystem objects, no block-level verification, file sizes can be truncated to 32 bits, timestamps unreliable, ≥4 GB risky. Use an SD card or external SSD with a reader.
- **iPhone transcodes HEIC/HEVC to JPEG/H.264 by default** before handing files to a PC (Settings → Apps → Photos → Transfer to Mac or PC). The transcode happens on the phone, so no copy path can avoid it — **under default settings what you get is not the original file.**
- **Formatting is gated on VM acceptance.** It is the only capability that irreversibly destroys data; acceptance is not performed on the development machine.
- **The portable build relies on the system WebView2.** The installer bundles it offline; the portable zip does not.

Full research on how each device class connects: [`docs/source-devices.md`](docs/source-devices.md).
Cross-layer command and event contract: [`docs/facade-contract.md`](docs/facade-contract.md).

## How this repo works

Three disciplines. (`openspec/` is a submodule pointing at a private repo; readers without access won't have it. The gist of the three disciplines is below.)

- **SDD** — OpenSpec. No propose→apply in one shot; human review is a gate that cannot be skipped
- **TDD** — spec-anchored, Detroit style. Every spec scenario has a same-named test; no mocked filesystem, real temp directories; adversarial tests are first-class
- **Dual-track testing** — anything that formats, needs admin, or touches physical devices goes on the danger track behind three gates, and **is never run on the development machine**. Registered in [`docs/danger-tests.md`](docs/danger-tests.md)

Before shipping, every item R1–R14 in [`docs/release-checklist.md`](docs/release-checklist.md) must pass. One unchecked box means no release.

## Licence

MIT. See [LICENSE](LICENSE).
