# Lapis/Nimbus "fully switches to local" — research (2026-11-21)

## Report
"Instead of just routing to Lapis, the target model is fully switched over to
lapis, rather than keeping Nimbus priorities."

## Findings (traced)

1. **Both virtual lanes have an unconditional continuation stick.**
   - Lapis: `resilient-default/index.ts:700`
     `request.reason === "continuation" && previous && previous.id !== "lapis"`
     → `return { model: previous }`.
   - Nimbus: same shape at `:1175` (`previous.id !== "nimbus"`).
   Rationale (documented at :99): "continuation stick to the previous physical
   model (tool-loop cache)". Keeping weights warm mid tool-loop is real value —
   the defect is that the stick is UNCONDITIONAL.

2. **Consequence for Nimbus (the operator's report).** Pick a lane that routes
   ONE turn local (cloud lanes cooling, budget exhaustion, contention, a user
   turn that fell to local) and every later continuation inherits the LOCAL
   physical model — even after cloud capacity returns. Nimbus stops being
   cloud-first for the rest of the session; only a `user` turn re-decides.
   The operator's "target model is fully switched over to lapis" is exactly
   this: the *physical* model sticks, and the virtual lane never re-asserts its
   priority.

3. **Same defect feeds yesterday's contention bug.** Lapis's stick returns
   BEFORE `decideTier`, so the fresh admission probe / contention mark added in
   5a7b778 cannot influence continuations at all — a contended local slot keeps
   being reused for the whole tool loop.

4. **Evidence from the live ledger.** `route-ledger-20261121.jsonl`: 644
   entries, **572 local vs 66 cloud, all but two with `reason:"continuation"`**
   — the tier is decided once per user turn and then frozen for the loop.

5. **Not the cause (checked and cleared).**
   - `adaptive-router` guards virtual models (`:350`, `:598`) — it never
     `setModel`s under lapis/nimbus.
   - `llama-cpp-provider` startup sync is bounded (60 s) and only arms when the
     provider is local/unknown at `session_start`; it announces itself so the
     router does not read it as an operator pin.
   - `phase-model` only switches when a phase model is tagged — no
     `phase-model.json` exists here.
   - `coupling` switches only on an explicit `/coup`.

## Proposed fix

Make the stick CONDITIONAL on the lane's own policy — "keep the same physical
model only while it is still the right tier for this lane":

- **Nimbus**: stick only when `previous` is a CLOUD model. A local `previous`
  re-decides (cloud-first is the operator's explicit priority).
- **Lapis**: stick for a cloud `previous` (warm cache); for a local `previous`
  stick ONLY while the slot is uncontented — if the probe/mark says contended,
  re-decide so the spill can happen mid-loop.
- A "stick" that survives must still publish its decision (unchanged).

## Invariants (draft)

- I1 parity: with `previous` on the lane's preferred tier, decisions are
  byte-identical to today (the common case: tool loops on the same tier).
- I2: Nimbus never serves a continuation on a local model when cloud capacity
  exists and privacy allows.
- I3: Lapis never serves a continuation on a contended local slot when cloud
  capacity exists.
- I4: a lane never switches tier mid-loop *without* a reason recorded in the
  ledger (no silent flip-flop: the re-decision must publish the new reason).
- I5: cache-affinity cap accounting is unchanged for stick decisions (a stick
  is not a switch).
