# Operations guide

How to run cirrus in production: process layout, configuration, and
day-2 maintenance.

## Process layout

A typical beamline deployment runs three or four cirrus processes:

```text
[ qs-manager ]   — control-plane (REQ/REP) + document fan-out (PUB)
[ frame-source ] — D21 acquisition process per detector (optional)
[ tiled (Python) ] — catalog HTTP server (optional)
[ kafka broker ] — durable doc bus (optional)
```

`cirrus qs-manager` is the always-on daemon. The rest are
deployment-specific: small sites can run everything in one process
(DocumentRouter + sinks attached directly to the engine inside qs).

## Service unit (systemd)

```ini
[Unit]
Description=cirrus queueserver
After=network-online.target

[Service]
Type=simple
User=acquire
Environment=EPICS_CA_ADDR_LIST=192.168.50.255
Environment=EPICS_CA_AUTO_ADDR_LIST=NO
Environment=RUST_LOG=info,cirrus_qs=debug
ExecStart=/usr/local/bin/cirrus qs-manager \
            --control   tcp://*:60615 \
            --documents tcp://*:60625 \
            --metrics   127.0.0.1:9090
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
```

Run `cirrus doctor` from the same shell environment as the unit
file to confirm EPICS / Tiled / Kafka are reachable before
starting the unit.

## State directory

cirrus persists a small amount of state under
`$XDG_CONFIG_HOME/cirrus` (or `~/.cirrus` on macOS):

```text
~/.cirrus/
├── config.toml          # default RPC ports, tiled url, kafka brokers
├── runs.jsonl           # rolling run-history index (uid, start, stop, exit)
├── profiles/            # named device-table snapshots
└── tokens/              # auth tokens for outbound HTTP (Tiled, Kafka)
```

`cirrus migrate` walks this directory and runs versioned migration
steps. Today the migration list is empty; the entry point lands so
future schema breaks have a single owner.

## Logging

cirrus uses `tracing` with the `RUST_LOG` env var:

```sh
RUST_LOG=info                                  # default
RUST_LOG=info,cirrus_engine=debug              # engine internals
RUST_LOG=info,cirrus_qs=trace                  # full RPC trace
```

For structured logs to a file:

```sh
RUST_LOG=info cirrus qs-manager 2>> /var/log/cirrus.log
```

Use `tracing-subscriber`'s JSON format if you ship logs to Loki /
ELK — the default formatter is human-readable.

## Metrics

`/metrics` (Prometheus exposition format) is enabled by passing
`--metrics ADDR` to qs-manager (binary built with the `metrics`
feature). See [Optional features](./features.md#observability) for
the metric list.

Suggested alert rules:

```yaml
- alert: CirrusQsErrorRateHigh
  expr: rate(cirrus_qs_rpc_errors_total[5m]) > 0.5
  for: 5m

- alert: CirrusQsQueueStuck
  expr: cirrus_qs_queue_depth > 100
  for: 10m
```

## Backup & restore

The state directory is small (kilobytes) and rsync-friendly. The
authoritative run history lives in your downstream Tiled / Kafka /
JSONL sink, not in cirrus state. Treat `~/.cirrus/runs.jsonl` as
operational, not authoritative.

## Upgrades

```sh
systemctl stop cirrus
cargo install --path crates/cirrus      # or cp the binary in
cirrus migrate --apply                  # run any new schema steps
systemctl start cirrus
```

Document compatibility is part of the public contract: cirrus N+1
emits documents that are forwards-compatible with bluesky readers
that worked against cirrus N.

## Troubleshooting

| Symptom                                | Likely cause                                        |
| -------------------------------------- | --------------------------------------------------- |
| `cirrus doctor` warns on EPICS         | `EPICS_CA_ADDR_LIST` empty or unset                 |
| `qs-manager` exits with `address in use` | another process bound the control / doc port      |
| Documents arrive at consumer with gaps | consumer is not draining its 0MQ queue fast enough; raise SUB HWM |
| `re-pause` returns immediately but engine keeps running | check engine `state` via `status` RPC — pause arrives at next checkpoint, not mid-Msg |
| HDF5 file is empty after a run         | check `cirrus_qs_documents_total{name="datum"}` — frames may be writing to a different sink |

For deeper diagnosis, raise `RUST_LOG=cirrus_qs=trace` for the
duration of one failing scan and capture the resulting log.
