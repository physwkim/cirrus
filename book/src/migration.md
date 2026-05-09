# Migration from bluesky

cirrus is a drop-in replacement at the **Document** boundary. A site
running bluesky today moves to cirrus by replacing one piece at a
time, never the whole stack at once.

## Where cirrus plugs in

```text
                       bluesky / Python                cirrus / Rust
Plan source            generators (bp.*) / Lua coro    cirrus_plans::* / Lua coro
RunEngine              bluesky.RunEngine               cirrus_engine::RunEngine
Devices                ophyd / ophyd-async             cirrus_devices, cirrus-protocols-async
Backends               pyepics / aioca / p4p           epics-ca-rs / epics-pva-rs
Document plumbing      Publisher (ZMQ)                 ZmqDocumentSink
                       TiledWriter                     TiledSink + tiled-rs
                       suitcase-jsonl                  JsonlSink
                       bluesky-kafka                   KafkaDocumentSink
                       NDFileHDF5 (in IOC)             Hdf5FrameSink (cirrus-stream)
queueserver worker     bluesky-queueserver             cirrus-qs (drop-in)
queueserver manager    bluesky-queueserver             unchanged (still Python)
Catalog browse         databroker / Tiled              tiled-rs / Python tiled
Live plot              BestEffortCallback              [open]
```

## Three migration patterns

### 1. Keep bluesky-queueserver, swap the worker

You're running `bluesky-queueserver` with a Python worker today. Swap
the worker for `cirrus qs-manager`:

```sh
# Before
start-re-manager --kafka-server localhost:9092 ...

# After
cirrus qs-manager --control tcp://*:60615 --documents tcp://*:60625
```

The QS manager (and your existing 0MQ JSON-RPC clients, web UI,
queue management) keep working. cirrus-qs implements the same RPC
surface (~30 methods, see
[cirrus-qs/src/dispatch.rs](https://github.com/physwkim/cirrus/blob/main/crates/cirrus-qs/src/dispatch.rs)).

Documents fan out from cirrus-qs's PUB socket to the same
`RemoteDispatcher`s your downstream consumers already use.

### 2. Keep bluesky.RunEngine in Python, swap the document plumbing

Use cirrus's sinks from Python via cirrus-py (M7 — deferred). For
now, the inverse works: run cirrus's RunEngine in a small Rust
helper binary, ZMQ-publish documents to your Python
`RemoteDispatcher` setup. Same wire format as bluesky.callbacks.zmq.

### 3. Native cirrus end-to-end

For new beamlines that don't have a bluesky / ophyd commitment yet:

```sh
cirrus repl --init beamline_devices.lua    # interactive scans
cirrus qs-manager                          # production worker
```

Devices in Rust use `#[derive(Device)]` from `cirrus-derive`:

```rust
use cirrus::ophyd_async::*;

#[derive(Device)]
pub struct Motor<B> {
    #[signal(rw, "{prefix}.VAL")]                pub setpoint: Signal<f64, B>,
    #[signal(ro, "{prefix}.RBV", kind = hinted)] pub readback: Signal<f64, B>,
    #[signal(rw, "{prefix}.VELO", kind = config)] pub velocity: Signal<f64, B>,
}
```

## Plan code translation

cirrus-plans mirrors `bluesky.plans` 1:1 by name. Direct ports:

| bluesky                          | cirrus                              |
| -------------------------------- | ----------------------------------- |
| `bp.count(dets, n)`              | `cirrus_plans::count(dets, n)`      |
| `bp.scan(dets, m, a, b, n)`      | `cirrus_plans::scan(...)`           |
| `bp.list_scan`, `rel_list_scan`  | `cirrus_plans::list_scan`, `rel_list_scan` |
| `bp.grid_scan`, `rel_grid_scan`  | `cirrus_plans::grid_scan`, `rel_grid_scan` |
| `bp.spiral_*`                    | `cirrus_plans::spiral_*`            |
| `bp.adaptive_scan`               | `cirrus_plans::adaptive_scan`       |
| `bp.tune_centroid`               | `cirrus_plans::tune_centroid`       |
| `bp.fly`                         | `cirrus_plans::fly`                 |
| `bp.ramp_plan`                   | `cirrus_plans::ramp_plan`           |
| `bp.log_scan`                    | `cirrus_plans::log_scan`            |
| `bps.*` (one-shot Msg helpers)   | `cirrus_plans::stubs::*`            |
| `bpp.run_wrapper`                | `cirrus_plans::preprocessors::run_wrapper` |
| `bpp.subs_wrapper`               | (no-op alias — see note below)      |
| `bpp.relative_set_wrapper`       | `cirrus_plans::preprocessors::relative_set_wrapper` |
| `bpp.baseline_wrapper`           | `cirrus_plans::preprocessors::baseline_wrapper` |
| `bpp.contingency_wrapper`        | `cirrus_plans::preprocessors::contingency_wrapper` |
| `bpp.finalize_wrapper`           | `cirrus_plans::preprocessors::finalize_wrapper` |
| `bpp.configure_count_time_wrapper` | `cirrus_plans::preprocessors::configure_count_time_wrapper` |

> `subs_wrapper` is documented in cirrus as a no-op for parity. The
> recommended replacement is `re.subscribe(cb)` at engine creation
> time; cirrus has no equivalent of bluesky's per-run
> `temp_callback_ids` swap.

## Document compatibility

cirrus emits the bluesky event-model 1.x document shape verbatim,
serialized as either JSON (default) or msgpack:

- `RunStart` / `EventDescriptor` / `Event` / `EventPage`
- `RunStop`
- `Resource` / `Datum` / `DatumPage`
- `StreamResource` / `StreamDatum`

A Python `RemoteDispatcher` configured with
`deserializer=msgpack.unpackb` consumes them unchanged; ditto
`databroker` if its catalog is wired to a Tiled / suitcase-jsonl
backend.

## What's intentionally different

`doc/03-runengine.md` lists the few places cirrus diverges from
`bluesky.run_engine.RunEngine`:

- `msg_hook` / `state_hook` / `waiting_hook` → `tracing` spans +
  broadcast subscribers.
- string command + dictionary registry → typed `Msg` enum +
  `Msg::Custom { name, payload }` escape hatch.
- `_run_permit: asyncio.Event` → `tokio::sync::Notify` +
  `AtomicState`.

These are equivalents, not omissions. See doc/03 for the full
table.

## What's deferred

cirrus does not (yet) ship a Python class for `cirrus.RunEngine` —
that's the M7 PyO3 layer in `doc/10-roadmap.md`. Until it lands,
the migration path is "cirrus binary on the IOC host, Python on the
analysis side, ZMQ between them."
