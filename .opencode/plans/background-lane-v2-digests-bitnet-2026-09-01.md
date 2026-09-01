# Background Lane v2 — Architectural Digests + Bitnet/Networked Workers

**Date:** 2026-09-01
**Status:** PROPOSAL + spec. Builds on the adopted dual-slot result
(`dual-slot-background-lane-2026-08-31.md` §6b: 1.51× parallel, zero
foreground stall) and the context-efficiency record (10:1 discard).

## 1. Architectural digests — the value case

Reads are the biggest remaining context cost. Repo-map gives *structure*
(where things are, PageRank-ranked); only reading gives *semantics*
(invariants, callers, which half is legacy). Digest cards deliver ~80% of
semantic value for ~5% of the tokens, and the background agent is strictly
better positioned to write them: it can read generously and compress hard on
idle cycles, while the foreground agent must stay token-frugal.

Expected payoff: replacing even half of all reads pushes the discard ratio
from ~10:1 toward ~15:1, and the saving compounds (evicted reads stop
propagating forward).

**The trap: staleness.** A stale digest is poison — the model trusts it.
Mitigation is algorithmic, not disciplinary (see §3): every card carries a
validity predicate over file content hashes, checked by pure code before
injection. Cards are a cache, not documents.

**Hit-rate risk:** pre-read neighbors only pay off if the agent's next need
matches. Mitigation: only pre-digest modules that are BOTH repo-map-ranked
AND adjacent to the current edit cluster. Misses cost idle cycles only.

## 2. Bitnet on CPU / a networked PC — the value case

- **Free capacity:** a spare PC running a Bitnet-class ternary model on CPU
  costs the cluster zero VRAM, zero GPU bandwidth, zero contention. For
  idle-harvesting workloads, "free and adequate" beats "perfect and
  contended".
- **The math fits old hardware:** ternary/LUT CPU inference (bitnet.cpp,
  T-MAC approach) reduces to memory bandwidth; DDR3/DDR4 + a 1–4B model
  plausibly yields 10–20 t/s — plenty for cards and summaries.
- **The ceiling is quality:** Bitnet-tier models are not diff reviewers or
  call-path tracers. Division of labor: **networked Bitnet = bulk compression
  and triage; local 27B slot = judgment work.**
- **Anti-pattern:** remote speculative decoding (draft on PC, verify local) —
  unsupported by llama.cpp and RTT-bound. Use the supported pattern: the PC
  runs its own llama-server; Officina couples to it as a third provider via
  the existing `couplings.json` mechanism, pinned to specific job classes.

Job-class routing (proposed):
| job class | worker |
|---|---|
| diff review, fix-racing, call-path scouting | local slot 0 (27B, idle-gated) |
| digest drafting, memory-curator triage, card-validity rechecks | networked Bitnet (CPU) |
| conversation summarization / compaction | mellum2 small lane (unchanged — needs the transcript) |

## 3. Digest pipeline spec (v2)

**Card format** (`.pi/background/<session>/digests/<module>.json`):
```json
{
  "module": "src/critical/path.rs",
  "covers": [{"path": "...", "hash": "sha1:..."}, ...],
  "card": "10-line digest: exports, key invariants, callers, TODOs",
  "generated": "2026-09-01T12:00:00Z",
  "worker": "slot0|bitnet"
}
```

**Validity:** before injection, re-hash every covered file; any mismatch →
card retired (silently; the lane may re-digest when idle). Checker is pure
code (house pattern: edit-churn) — the model never decides validity.

**Generation:** job enqueued when (a) repo-map ranks the module high, (b) it
is within 1 hop of the current edit cluster, (c) no valid card exists,
(d) lane idle-gated as today. Prompt shares the byte-stable prefix rule for
prompt-cache reuse.

**Injection:** served through the existing knowledge-inject scoring path —
digest cards are just another knowledge entry with high local relevance.
Pulled, never pushed (anti-noise clause, mining audit 2026-08-31).

**Worker routing:** `worker` field on the job; the lane POSTs to whichever
coupling the job class pins. v2 keeps everything on slot 0; the Bitnet
provider is v2.5 (needs the networked host set up).

## 4. Proposed next steps, in order

1. **Live-fire background-lane v1** (user, ~10 min): a working session with
   real edits → `grep lc-bg ~/.vitriol/officina/state/events.jsonl` → first
   findings cards. Gate: cards must be non-trivial before v2 builds on it.
2. **Digest v2 on slot 0** — card format + hash validity + knowledge-inject
   serving (all local, no new hardware).
3. **Bench-dual-slot regression** into the engine-change checklist (already
   written: `scripts/bench-dual-slot.py`).
4. **Set up the networked PC** — bitnet.cpp (or llama.cpp with a ternary
   GGUF) + llama-server on :8288; add coupling; route digest drafting to it
   (v2.5). Measure t/s before trusting it with any job class.
5. **Churn investigator** (slot 0) and **structured-output** verdict cards,
   after 1–2 prove card quality.
6. Standing debts (unchanged): 2 vitriol-tui test failures, HTTPS
   credentials, glyph font fallbacks.
