# Optional features

cirrus is a Cargo workspace. Most opt-in functionality lives behind
feature flags so the default build stays small and dependency-free.

## Document sinks

| Crate              | Feature  | Pulls in                              | Use when                              |
| ------------------ | -------- | ------------------------------------- | ------------------------------------- |
| `cirrus-callbacks` | `zmq`    | libzmq + rmp-serde                    | bluesky `RemoteDispatcher` consumers  |
| `cirrus-callbacks` | `tiled`  | tiled-client (HTTP)                   | Tiled catalog ingestion               |
| `cirrus-callbacks` | `kafka`  | pure-Rust `kafka` crate               | Kafka topics, no librdkafka           |

```sh
cargo build -p cirrus-callbacks --features zmq,tiled,kafka
```

## Frame sinks (cirrus-stream)

| Feature  | Pulls in              | Use when                                     |
| -------- | --------------------- | -------------------------------------------- |
| `hdf5`   | rust-hdf5 (pure Rust) | NeXus-flavored HDF5 detector files           |
| `pva`    | epics-pva-rs          | NTNDArray monitor source                     |

`Hdf5FrameSink::new("det", "/data/run.h5", payload_size)` writes
into `/entry/instrument/<name>/data` (chunked, optional gzip) and
emits Resource + Datum docs pointing at the file path. The
`PvaMonitorSource` subscribes to a PVA NTNDArray PV and pushes
frames into a `FramePipe` that fans out to one or more sinks.

## EPICS backends

| Crate                       | Feature | Behavior without feature           |
| --------------------------- | ------- | ---------------------------------- |
| `cirrus-backend-epics-ca`   | `real`  | Stub backend that errors on call   |
| `cirrus-backend-epics-pva`  | `real`  | Stub backend that errors on call   |

```sh
cargo build -p cirrus-backend-epics-ca --features real
```

The stub-by-default lets the rest of the workspace compile cleanly
on systems without EPICS. CI build-tests both the stub and the
`real` paths; live IOC integration testing is on the roadmap.

## Lua read-side surface

| Crate        | Feature | Adds                                       |
| ------------ | ------- | ------------------------------------------ |
| `cirrus-cli` | `tiled` | `tiled.from_uri(url)` Lua global + methods |

```sh
cargo build -p cirrus-cli --features tiled
```

Inside the REPL:

```lua
local cat = tiled.from_uri("http://localhost:8000")
for _, k in ipairs(cat:keys()) do print(k) end
local run = cat:get("scan_42")
print(run:metadata())
```

All HTTP calls run on cirrus's tokio runtime; the REPL thread
re-enters mlua's reentrant lock, so calls inside Lua plans are
safe.

## Observability

| Crate       | Feature   | Adds                                  |
| ----------- | --------- | ------------------------------------- |
| `cirrus-qs` | `metrics` | Prometheus `/metrics` HTTP listener   |

```sh
cirrus qs-manager --metrics 127.0.0.1:9090
# build first with: cargo build -p cirrus-qs --features metrics
```

Currently exported:

- `cirrus_qs_rpc_calls_total{method=...}`
- `cirrus_qs_rpc_errors_total{method=...}` (when wired)
- `cirrus_qs_queue_depth` (gauge)
- `cirrus_qs_runs_total{exit_status=...}`
- `cirrus_qs_documents_total{name=...}`

Scrape with the standard Prometheus config:

```yaml
scrape_configs:
  - job_name: 'cirrus-qs'
    static_configs:
      - targets: ['localhost:9090']
```
