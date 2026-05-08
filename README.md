# cirrus

Rust DAQ RunEngine + Device framework.
A Rust-compatible reimplementation of bluesky + ophyd-async, with EPICS CA/PVA backends
built directly on `epics-rs` (no Python shim), and a future extension path for rogue
(SLAC FPGA DAQ).

## What it is, in one line

> **cirrus is "bluesky + ophyd + ophyd-async, rewritten in Rust", with a single async
> core and two co-equal user-facing API surfaces — `cirrus::ophyd_async` (default) and
> `cirrus::ophyd` (sync facade).**

## Status

Design phase. Implementation has not started.

## Two co-equal API surfaces

```rust
// async (default) — ophyd-async style
use cirrus::ophyd_async::*;
let val = motor.read().await?;
motor.set(1.0).await?;

// sync — ophyd style, equally supported
use cirrus::ophyd::*;
let val = motor.read()?;
motor.set(1.0)?;
```

Both surfaces share the same `Device` types underneath. The sync API blocks on
the cirrus tokio runtime; the async API runs in caller's task. Plans, callbacks,
and Documents are identical between the two.

## EPICS backend

Both CA and PVA are implemented in `epics-rs`:

- `epics-ca-rs` for Channel Access
- `epics-pva-rs` for PV Access

cirrus depends on these crates directly. There is no Python `pyepics` / `caproto` /
`ophyd-epicsrs` shim layer.

## Design documents

Read in order:

1. [`doc/00-vision.md`](doc/00-vision.md) — what / why / where the name comes from
2. [`doc/01-architecture.md`](doc/01-architecture.md) — 4-layer architecture + 3 sealed traits
3. [`doc/02-event-model.md`](doc/02-event-model.md) — Document schemas via `typify`
4. [`doc/03-runengine.md`](doc/03-runengine.md) — message loop, Bundler, Suspender, Checkpoint
5. [`doc/04-devices.md`](doc/04-devices.md) — protocol traits, Signal, StandardDetector, dual API
6. [`doc/05-streaming.md`](doc/05-streaming.md) — FrameSource/Sink + Phase 2 rogue integration
7. [`doc/06-rules.md`](doc/06-rules.md) — K1–K12 rules from kodex bug patterns
8. [`doc/07-milestones.md`](doc/07-milestones.md) — M0–M7 milestones
9. [`doc/08-decisions.md`](doc/08-decisions.md) — locked decisions + open questions
10. [`doc/09-references.md`](doc/09-references.md) — paths to upstream source trees

## One promise

The 3 sealed traits — `SignalBackend`, `FrameSource`/`FrameSink`, `DetectorWriter` —
are decided in Phase 1 and **never break afterwards**.
Everything else may evolve freely.
