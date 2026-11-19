# Cache-cap verification vs operator's 185 GB report — 2026-11-19 evening

## Question
Operator hit Shift+W because "the cache was at 185 GB" — wasn't the cap 40 GB?

## Findings (all measured, PLAN mode)

### The 40 GiB disk cap IS enforced — evidence chain
1. **Delivery**: `scripts/vitriol` exports `VITRIOL_KV_CACHE_MAX_GB=40`; present in
   the live engine's `/proc/1531332/environ`. Engine reads it
   (server-task.cpp:2463 `server_prompt_cache_disk_cap_bytes`).
2. **Enforcement**: live log `/tmp/opencode/vitriol_gen.log` shows 4 trim sweeps
   this boot — `41.4/41.5 GiB + incoming exceeds 40.0 GiB, evicting
   least-recently-addressed` → `directory now 39.8 / 37.2 / 33.5 / 36.0 GiB`.
   Counter: `kv_cache_evicted_total = 17`.
3. **So the directory peaked at ~41.5 GiB** (slightly over, then pulled back —
   trim fires only on spill/fetch, so a burst can overshoot by one incoming
   file before the sweep; by design, ≤ ~1 state ~2.7 GiB over).

### The 185 GB number does not match the engine cache anywhere
- kvtier per logs: peak 41.5 GiB. Now (post-wipe): 2.5 GB.
- TUI cache panel (`tui/src/tui/cache.rs scan()`) reads ONLY the cache dir —
  it cannot print 185 from these numbers.
- `/scratch` as a whole: **188 GB used** — suspiciously close to "185 GB".
  But its tenants are NOT the cache: models 95 G, lora_prestop 17 G,
  lora_a1c 17 G, pacman 11 G, briev-builds 11 G, sdb-mft.img 8.1 G,
  SteamLibrary 6.6 G, a3rect_smoke 6.5 G. The cache is a rounding error here.

### What Shift+W actually did
`cache_wipe(None)` → deletes every non-slot0.bin file in kvtier. It wiped ~
35 GiB of warm cache states — the re-prefill cost you saw next session was
real, but the freed space was kvtier only (~35 GiB), not 185 GB.

### Possible explanations for the 185 GB reading (need operator input)
- **A. `/scratch` df fill misattributed to the cache** (188 vs 185 close). If
  some readout (governor panel, df, or a plan doc) showed disk fill, it was
  lora/steam/pacman, not the KV cache.
- **B. A historical kvtier peak the current log doesn't cover** — the log starts
  at 13:50 today (this boot). Earlier boots (Nov 17-19 morning) could have
  accumulated more, BUT: entries persist across restarts, and trim runs on the
  next spill, so a 185 GB directory would have been trimmed at the first spill
  after 13:50 — log says it was only 41.4 GiB then. **So B is excluded unless
  the directory was big AND no spill happened** — spills happened constantly.
- **C. RAM-side accounting misread**: the RAM prompt cache cap is 16 GiB
  (cram=16384); the engine RSS showed 0.9 GiB + 16.4 GB swapped. No 185 there.

## Gaps found (fixable later, code mode)
1. **No boot-time / periodic trim**: trim only on spill + fetch. If the cap is
   lowered between boots (or files added externally), the directory stays over
   until the next spill. A boot sweep (or before the readout) would close it.
2. **Panel scope**: the cache panel shows the dir total but the operator
   experience conflated it with disk fill — a "/scratch disk" line next to the
   cache budget line would disambiguate (df is one stat() call).
3. **Log rotation**: trim warnings only in the engine log; journal carries
   nothing. Fine, but the fingerprint doctrine wants cap state in /metrics —
   `kv_cache_evicted_total` exists; consider adding `kv_cache_dir_bytes` gauge
   so the panel and the engine agree on the same number from one source.

## Recommended operator action
- Do NOT treat the cache as the /scratch problem. If /scratch is the concern:
  lora_prestop (17G) + lora_a1c (17G) + pacman cache (11G) + sdb-mft.img (8G)
  are the levers — 53 GB recoverable without touching the cache.
- Keep cache_max_gb = 40. The cap works; overshoot is bounded by one state.

## Where was 185 GB displayed? (open question for the operator)
TUI cache panel line, a `df` run, the governor, or elsewhere — this decides
between explanations A and B.
