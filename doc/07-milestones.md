# 07 — Milestones

## Phase 1 (bluesky-in-Rust)

The word "rogue" appears 0 times in code through M7. M5 is the first time
`FrameSource` is exercised end to end, validating the trait surface that Phase 2 will
reuse.

| # | Deliverable | Acceptance |
|---|---|---|
| **M0** | `cirrus-event-model` (typify auto-gen) + `cirrus-core` + `cirrus-protocols-async` + `cirrus-protocols-sync` | Round-trip test: Python event-model JSONL → Rust deserialize → re-serialize → diff empty. The 11 protocol traits compile (no impls yet). |
| **M1** | `cirrus-backends/soft` + `cirrus-engine` (5-msg subset: open_run/read/set/trigger/close_run) + `count` plan + `JsonlSink` | `count(soft_det, 5)` emits a 9-document stream byte-equal to the bluesky reference output. |
| **M2** | `cirrus-backends/epics-ca` (using `epics-rs/crates/epics-ca-rs`) + `EpicsMotor` device | `mv(motor, 1.0)` against a real IOC. Sync facade: `motor.set(1.0)?.wait(Some(2 sec))?`. |
| **M3** | `StandardDetector<C, W>` + soft `Hdf5Writer` + `scan` plan | A 5-point step scan emits 5 HDF5 frames + matching StreamResource/StreamDatum docs. Tiled can read the run. |
| **M4** | checkpoint / pause / resume / suspender + SIGINT 3-tap | Pause restores monitors. Resume replays from checkpoint. SIGINT taps {pause, abort, halt} confirmed. AbortOnDrop audit passes. |
| **M5** | `cirrus-backends/epics-pva` (using `epics-rs/crates/epics-pva-rs`) + `PvaMonitorSource: FrameSource` + `Hdf5Sink: FrameSink` + `fly` plan | Fly scan against a PVA-monitored areaDetector PV. NDArray frames flow zero-copy into HDF5. K-rules audited. |
| **M6** | `cirrus-cli` REPL + `BestEffortCallback` (table only) | A user runs an interactive session that mirrors a typical bluesky-IPython workflow. |
| **M7** | (optional) `cirrus-py` PyO3 adapter | A bluesky `count` plan written in Python imports cirrus device classes and runs on the cirrus RunEngine. K10 enforced. |

## Phase 2 (rogue addition, only when needed)

| # | Deliverable | Acceptance |
|---|---|---|
| **P2-A** | `cirrus-backends/rogue/ctrl` — ZMQ Variable backend impl `SignalBackend` | A rogue Tree Variable can be read/written through cirrus's `Signal<T>`. K11 enforced. |
| **P2-B** | `cirrus-stream/sources/rogue_dma` impl `FrameSource` | A rogue DMA-backed detector emits Frames into the same FramePipe used by PvaMonitorSource in M5. Plan code is unchanged. |

No trait change is needed for either Phase 2 milestone. cirrus-engine and cirrus-plans
crates do not even need to be rebuilt — they depend on traits, not on backend impls.

## What "M0 done" looks like, in commits

The first PR of cirrus produces:

1. `cirrus/Cargo.toml` (workspace, members: event-model / core / protocols-async / protocols-sync).
2. `cirrus/crates/cirrus-event-model/build.rs` — typify pulls schemas from `schemas/`.
3. `cirrus/crates/cirrus-event-model/src/lib.rs` — `Document` enum + re-exports.
4. `cirrus/crates/cirrus-event-model/tests/round_trip.rs` — the JSONL acceptance test.
5. `cirrus/crates/cirrus-core/src/{reading.rs, status.rs, msg.rs, plan.rs, runtime.rs}` skeletons.
6. `cirrus/crates/cirrus-protocols-async/src/lib.rs` — 11 trait signatures, no impls.
7. `cirrus/crates/cirrus-protocols-sync/src/lib.rs` — 11 sync trait signatures + blanket impls over async via `block_on`.

`cargo test` passes — round-trip test green, type checks clean.

## Out-of-band tracks (parallel to milestones)

Each can be picked up at any milestone without disturbing the main path:

- **Documentation site** — render `doc/*.md` via `mdbook` after M2 (gives users
  something to read while M3 lands).
- **CI** — `cargo test` + `cargo clippy -- -D warnings` + K-rule lints (custom clippy
  via `dylint` if appetite exists). After M0.
- **Soft IOC harness** — using `epics-rs/examples/ophyd-test-ioc`, drive M2/M5 tests
  without a real beamline. After M2.
- **Performance baseline** — `criterion` benches for plan-loop overhead and
  document-fan-out throughput. After M4.
