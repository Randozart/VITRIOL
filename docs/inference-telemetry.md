# Inference telemetry

The dashboard's completed-task card retains prefill/decode durations and draft
counts from matching engine slot/task logs. Cached prompt tokens are retained
only if that task was observed while active. A fast request can therefore have
known timings and unknown cached tokens. Records begin after the dashboard
establishes engine identity and reset across process changes, log rotation or
unread intervals.

`TTFT mean` is the in-tree engine's mean time from task posting to the first
sampled token, since process start. It includes server queue time. It requires
both `llamacpp:ttft_seconds_sum` and a positive
`llamacpp:ttft_seconds_count`. Reference Kintsugi lacks these series, so its
native metric remains unavailable.

`KV I/O` shows the latest completed checkpoint save/restore operation reported
by autosave, with its byte count and engine-reported duration. Records must
match the served PID and port. This duration includes serialization and file
I/O, and restore can include the draft state. Byte counts describe the target
state file and can exclude draft sidecars. These figures do not isolate GPU
DMA time or measure ongoing KV tier traffic. An asynchronous save acknowledgement
is excluded. The changed autosave script takes effect when that service next
starts; the TUI does not restart services automatically.

The benchmark measures client time from sending one native streaming completion
request to receiving its first nonempty generated content. It excludes role-only
and empty events. This includes transport and buffering, and can differ from
native TTFT. Final engine timings come from that same stream.

Run against an already launched isolated engine, passing its actual PID and
stdout log. The benchmark excludes production port 8279, checks process identity,
and records full engine arguments and the dashboard binary hash. For example,
with the isolated process's PID substituted below:

```sh
python scripts/benchmark_inference_telemetry.py \
  --base http://127.0.0.1:18781 --pid ISOLATED_ENGINE_PID \
  --engine-log /tmp/telemetry/vitriol_gen.log \
  --dashboard "$HOME/.local/bin/vitriol-tui" \
  --output /tmp/telemetry/results.json --pairs 16 --tokens 256 --checkpoint
```

Each pair alternates dashboard-off/on order. Both arms have the same settling
interval and an untimed warmup before measurement. The dashboard runs in a
125-column terminal with normal polling and rendering. The comparison measures
the whole dashboard's effect on this workload, including telemetry polling.
`--checkpoint` additionally saves and restores slot 0 using the engine's configured
save directory. Without it, the benchmark only makes completion requests.

The report includes a deterministic paired-bootstrap interval for mean fractional
decode-time overhead. The below-1% criterion passes only when the interval's
upper bound is below 1%. This interval captures variation among the sampled
pairs; it does not remove display activity, clock drift or generalize a small
model's result to a 200K Kintsugi session. Generated text hashes are diagnostic;
output identity is not a correctness requirement.
