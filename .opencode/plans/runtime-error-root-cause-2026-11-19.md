# Root Cause: "Cannot read properties of undefined (reading 'runtime')" — FOUND

**2026-11-19, PLAN-mode investigation. Status: root cause identified and live-reproduced; fix is one line (not applied — plan mode).**

## Verdict

`officina/.pi/extensions/_shared/engine-provider.ts` — `findEngineModel()` (A2, landed
2026-11-19 13:22) **detaches the `find` method from its object**:

```ts
const find = registry.find;          // ← method detached; `this` lost
...
const hit = find(provider, modelId); // ← called bare: this === undefined
```

`registry` is pi's `ModelRegistry` facade. Its `find` is
`find(provider, modelId) { return this.runtime.getModel(provider, modelId); }`.
Called detached, `this` is `undefined` → exactly
**`Cannot read properties of undefined (reading 'runtime')`**.

## Evidence chain

1. **Timestamps match**: A2 wrote `engine-provider.ts` at 13:22; the first error
   surfaced 13:17–13:18 (rounded the same edit window; the 13:17–13:20 errors I
   previously blamed on a stale binary were actually this bug via a
   mid-edit-tree load — the timestamps overlap the edit session).
2. **Live reproduction** (throwaway probe dir `/home/randozart/probe-rt`, carried
   extensions, `--model officina/lapis`): error reproduces; with instrumentation:
   ```
   STEP 1a THREW: TypeError: Cannot read properties of undefined (reading 'runtime')
       at find (chunk-44DLKSKE.js:715:18203)          // ModelRegistry.find
       at findEngineModel (engine-provider.ts:65:17)  // detached call
       at Object.route (resilient-default/index.ts)   // lapis route
       at async _ModelRuntime.resolveModel (chunk)
   ```
3. **Negative control**: a minimal virtual-model extension calling
   `ctx.modelRegistry.find(...)` **attached** works — full streaming, real upstream
   429 from a free lane. `ctx.modelRegistry.runtime` itself is `defined`. Only the
   detached call breaks.
4. **Why only some turns**: the error fires on the **lapis route path**
   (`findEngineModel(ctx.modelRegistry, "Lapis Occultus")` at the top of the
   route). `loadCandidates(ctx)` survived because it calls
   `ctx.modelRegistry.find(...)` directly (attached). Every `officina/lapis`
   turn dies → the "grind: model call failed" spam; the next route attempt that
   reaches a cloud lane is why the session limps along.

## Why the earlier diagnosis was wrong

I first blamed a stale deleted TUI binary. Refuted: the error crossed the
13:50 rebuild + restart (fresh binary, errors at 15:56/16:13/16:14). The
distinguishing fact I missed: **0 successful `pi-virtual` messages** in the
failing session — every lapis-routed turn failed, meaning the bug was in the
route chain itself, not session state.

## Fix plan (code mode, one line + test)

1. **`_shared/engine-provider.ts`** — call the method attached:

   ```ts
   const ids = [ENGINE_PROVIDER_ID, ...LEGACY_ENGINE_PROVIDER_IDS];
   for (const provider of ids) {
     const hit = registry.find?.(provider, modelId);  // attached; optional-chained
     if (hit !== undefined) return hit;
   }
   ```

2. **Regression test** (engine-provider.test.ts): fake registry whose `find`
   asserts `this === registry` (throws on detached call) — proves the fix and
   pins the behaviour. Negative control: current detached form fails the test.

3. **Audit for the same pattern**: grep extensions for detached method calls on
   pi facades (`const x = obj.method; x(...)` on `modelRegistry`, `sessionManager`,
   `ui`). (Quick scan suggested this is the only instance.)

4. **Verify**: re-run the probe repro (`--model officina/lapis`) against the
   fixed repo tree — expect a routed turn (or an honest routing error), never
   `reading 'runtime'`.

## Related follow-ups (separate)

- `/scratch` is **100% full** (375 G, 244 K free) — cleanup needed before any
  build work; my probe tarball at `/scratch/pi-152-probe` should be removed.
- pi: `package.json` pins 1.5.2 but `node_modules` has **1.5.0** — dependabot #31
  merged but `npm install` never ran.
- Throwaway probe dir `/home/randozart/probe-rt` can be deleted.
