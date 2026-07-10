# pathwx — network weather station

English | [日本語](README.ja.md)

**This tool doesn't ask "is it broken?" — it asks "is it getting worse?"**

pathwx (path + the aviation-weather "wx") repeatedly probes TCP connect quality to a
destination, accumulates loss rate / latency / jitter time series as plain-text JSONL,
and watches for a statistically significant deviation (z-score) from that target's own
historical baseline — catching gradual degradation long before anything actually breaks.

## Why this exists

Every other tool in pathvector-studio looks at a single moment or a single layer:

| Tool | What it looks at |
|---|---|
| [netblame](https://github.com/pathvector-studio/netblame) | Who's at fault for *this one* connection problem, right now (one-shot diagnosis) |
| [ashealth](https://github.com/pathvector-studio/ashealth) | BGP/RPKI/IRR/PeeringDB consistency for an AS operator (periodic checkup) |
| [driftmap](https://github.com/pathvector-studio/driftmap) | Topology diffs between LLDP snapshots (configuration drift) |
| [bgp-sentinel](https://github.com/pathvector-studio/bgp-sentinel) | Real-time BGP hijack detection |

Each of those looks at a single point-in-time state or a configuration diff. None of
them continuously tracks the **time-series trend** of "this path has been quietly
getting worse over the last week." Most ping-monitoring tools only support fixed
thresholds (e.g. alert if RTT > 100ms), which means constant false alarms on noisy
links and missed early warnings on links that are normally rock-solid.

pathwx judges each round against a historical baseline (mean + standard deviation) via
**z-score**, so the question becomes "is this normal *for this target*?" — the goal is
to catch the precursor to an outage, not just the outage itself.

## Try it in 5 minutes

```console
$ cargo build --release
$ ./target/release/pathwx init
Sample config file written: pathwx.toml

$ cat pathwx.toml
# ...
[[targets]]
name = "cloudflare-dns"
host = "1.1.1.1"
port = 443
interval_secs = 60
samples = 5
timeout_ms = 1000

# Run one round (appends to the log)
$ ./target/release/pathwx probe
TARGET                   SENT       OK      LOSS%    AVG(ms)
cloudflare-dns              5        5         0%       12.4

# After a few rounds of history have accumulated, compare the latest round to baseline
$ ./target/release/pathwx check cloudflare-dns
cloudflare-dns       [HEALTHY] loss=0% avg=13.1ms — within baseline range

# Show baseline stats and a recent-history sparkline from the log
$ ./target/release/pathwx report --target cloudflare-dns --window 20
=== pathwx report: cloudflare-dns ===
total rounds: 12
baseline (12 rounds): mean latency=13.20ms stddev=2.10ms mean loss=0.0%
last 12 rounds latency trend: ▂▃▁▄▂▃▂▁▃▂▄▂
latest round verdict: Healthy — within baseline range

# Long-running observation (Ctrl-C to stop; loops per each target's interval_secs)
$ ./target/release/pathwx watch
```

For CI/cron use, branch on `check`'s exit code:

```console
$ pathwx check cloudflare-dns; echo $?
0   # healthy, or insufficient data (not enough history is not treated as an error)
1   # degraded (deviation from baseline)
2   # unreachable (every attempt in the latest round failed) / runtime error
```

## Subcommands

| Command | What it does |
|---|---|
| `pathwx init [--out FILE] [--force]` | Write a sample config file |
| `pathwx probe` | Run one round against every target, print a summary, append to the log |
| `pathwx watch [--rounds N]` | Loop forever, probing each target on its own interval (foreground) |
| `pathwx check [target]` | Run one round, compare it to baseline, print the verdict, and set the exit code |
| `pathwx report --target NAME [--window N]` | Show baseline stats and a recent-trend sparkline from the log |

## Configuration (TOML)

```toml
# Where to store data (defaults to ~/.local/share/pathwx)
# data_dir = "/var/lib/pathwx"

# How many recent rounds to use for the baseline (defaults to the full history)
# baseline_window = 200

# z-score threshold for flagging degradation (default 2.0)
z_threshold = 2.0

# Minimum rounds of history required before a baseline is trusted
# (fewer than this and pathwx reports "insufficient data")
min_baseline_rounds = 5

[[targets]]
name = "cloudflare-dns"
host = "1.1.1.1"
port = 443
interval_secs = 60   # probe interval used by `watch`, in seconds
samples = 5           # TCP connect attempts per round
timeout_ms = 1000     # per-attempt connect timeout, in milliseconds
```

## Data format

Each target's log lives at `<data_dir>/<target>.jsonl`, one round per line. Since it's
plain text with no database involved, **you can commit it to git and review it, or read
it directly with `jq` or `tail -f`**.

```json
{"timestamp":"2026-07-07T11:00:00Z","target":"cloudflare-dns","host":"1.1.1.1","port":443,"attempts":5,"successes":5,"loss_rate":0.0,"mean_latency_ms":12.4,"jitter_ms":1.1}
```

## Degradation logic

1. Every attempt in the latest round failed → **UNREACHABLE**
2. Fewer than `min_baseline_rounds` of history exist → **INSUFFICIENT DATA** (a
   cold-start guard against false alarms)
3. Compute a mean + standard deviation baseline from history (all rounds excluding the
   latest, or the most recent `baseline_window` rounds)
4. The latest round's mean latency has a z-score beyond `z_threshold`, or the loss rate
   spikes well above baseline → **DEGRADED**
5. Otherwise → **HEALTHY**

The key idea: judge each target against *its own* past behavior instead of a fixed
threshold that has to be guessed per-network.

## Constraints and known limitations

- **TCP connect only — no ICMP or raw sockets.** Same constraint as netblame: it runs
  without elevated privileges, but that means it can't measure pure reachability to a
  destination with no open port. An ICMP option is tracked for a future release — see
  ROADMAP.md.
- **`watch` is a single-process foreground loop only.** Daemonizing it (systemd unit,
  detaching, etc.) is left to you; pathwx doesn't ship one yet.
- **No per-target scheduling.** `watch` loops on the *shortest* `interval_secs` across
  all configured targets rather than timing each one independently — a deliberate MVP
  simplification. Accurate per-target timers are a future improvement.
- **The baseline is a simple mean + standard deviation model.** It does not account for
  periodic patterns (time-of-day, day-of-week).
- **Single process, single machine.** No distributed observation or cross-site
  correlation.
- **JSONL logs grow without bound.** pathwx never rotates or truncates
  `<data_dir>/<target>.jsonl` — every `probe`/`watch`/`check` round appends a line
  forever. For a long-lived `watch` process this is an unbounded disk-usage growth
  path; rotate or prune the file yourself (e.g. a cron job that trims to the last N
  lines) until log rotation ships. See ROADMAP.md.
- **The default `data_dir` (`~/.local/share/pathwx`) is shared across all configs on a
  machine.** If you run pathwx from two different directories with two different
  `pathwx.toml` files that happen to define targets with the *same name*, they will
  write to and read from the same `<name>.jsonl` file and silently mix each other's
  history. Set an explicit `data_dir` per config (or use distinct target names) to
  avoid this footgun.
- **Only `check`'s exit code encodes health.** `probe`, `watch`, and `report` always
  exit `0` on success regardless of what they observe (degraded latency, loss, etc.) —
  they're informational commands, not health gates. If you're scripting a health check
  (CI, cron, alerting), use `check`; don't try to infer health from the exit code of
  the other subcommands.

## License

MIT License. See [LICENSE](LICENSE).

## Related projects

- [netblame](https://github.com/pathvector-studio/netblame) — one-shot diagnosis
- [ashealth](https://github.com/pathvector-studio/ashealth) — AS operations health checks
- [driftmap](https://github.com/pathvector-studio/driftmap) — network diagrams that don't rot
- [bgp-sentinel](https://github.com/pathvector-studio/bgp-sentinel) — real-time BGP hijack detection

See [ROADMAP.md](ROADMAP.md) for what's planned next.
