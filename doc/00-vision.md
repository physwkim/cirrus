# 00 — Vision

## What we are building

A Rust port of **bluesky + ophyd + ophyd-async** with EPICS CA/PVA backends built
directly on `epics-rs` (no Python shim).
A reserved extension surface so that **rogue** (SLAC FPGA DAQ) can be added in Phase 2
without forklift changes.

A single user, on a single RunEngine, can:

- drive EPICS motors / detectors / waveforms
- (Phase 2) drive direct-attached FPGA boards via rogue, in the same plan
- emit bluesky-compatible Document streams to Tiled / HDF5 / Kafka

## Two user populations, one core

cirrus is async on the inside but exposes **two co-equal** API surfaces:

| Module | Style | Origin | Users |
|---|---|---|---|
| `cirrus::ophyd_async` | async / await | ophyd-async (Python) | new code, `await` everywhere |
| `cirrus::ophyd` | sync, blocking | ophyd (Python) | scripts, REPL, ophyd-trained users |

The sync layer is **not a second-class facade**. It is a peer surface: the same
`Device` and `Signal` types appear in both, with method signatures translated.
Internally the sync methods drive the async ones via the cirrus tokio runtime.

This matches Python practice — bluesky `RE(plan)` is sync (runs asyncio in a worker
thread), ophyd is sync (uses CA dispatcher thread), ophyd-async is async. cirrus
unifies all three under one Rust async core.

## Why rewrite

| Issue | bluesky + ophyd | cirrus |
|---|---|---|
| GIL bottleneck in single process | Yes | None (async tokio) |
| Same language as IOC | Python ↔ C IOC boundary every time | Rust IOC (`epics-rs`) lives next door |
| EPICS protocol stacks | C `libca.so` + C++ `pvxs` | pure Rust `epics-ca-rs` + `epics-pva-rs` |
| Direct-attached hardware (rogue) | Hard to integrate | One trait impl, lands cleanly |
| Memory + cancellation safety | Human discipline | Compiler-enforced + K1–K12 rules |

## Where the name comes from

**cirrus** = high-altitude wispy cloud.

- Aligns with the NSLS-II sky/cloud naming convention (bluesky / nimbus / databroker / tiled).
- Conveys "light and fast" — fits the single-task message loop of the RunEngine.
- Searchable: Cirrus CI exists but in a different domain, low collision risk.

The earlier candidate `rdaq` was rejected because (a) it does not say *which* DAQ, (b)
it breaks the local `archiver-rs` / `epics-rs` suffix-rs convention, and (c) is unsearchable.

## Phase strategy

```
Phase 1: bluesky-in-Rust   (the word "rogue" appears 0 times in code)
   M0 ─► M1 ─► M2 ─► M3 ─► M4 ─► M5 ─► M6 ─► M7?
                                          │
                                          └── complete bluesky replacement here

Phase 2: rogue addition    (only when needed; 0 trait changes)
   P2-A: rogue ZMQ Variable backend  (impl SignalBackend)
   P2-B: rogue DMA frame source      (impl FrameSource)
```

Detailed breakdown in [`07-milestones.md`](07-milestones.md).

## Non-goals

- Re-implementing the full bluesky callback ecosystem (BestEffortCallback, LiveTable, LivePlot)
  in Phase 1. Tiled / JSONL / HDF5 sinks are enough.
- Replacing areaDetector C++ plugins. cirrus is a PVA-monitor *consumer*; IOC-side replacement
  belongs to `epics-rs/crates/ad-core-rs` and `ad-plugins-rs`.
- Replacing bluesky-queueserver / bluesky-httpserver. If needed later, separate crate.
