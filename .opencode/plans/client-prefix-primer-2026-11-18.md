# 2026-11-18 13:54 — Explicit client startup priming

Add an optional standard-library CLI/module that primes the exact shared chat
prefix through a resident OpenAI-compatible native engine before a consumer is
ready. Generic messages/model/URL inputs; no model recognition, daemon, background
scheduler or production launcher change. Use native one-token inference and discard
its output; do not add a fake dialogue to client history. Total deadline includes
health/template/tokenizer/completion I/O. Record argv and a fingerprint, input and
prefix hashes, actual cached/evaluated counts, truncation and failure diagnostics.
Avatar integration stays private, same20B/current voice/publicv9 unchanged. No
killall: the user's live-preservation instruction overrides the stale-server rule;
only isolated units may be stopped. Existing atime compile fix63cc63f is carried.

## Invariants

1. I1: no invocation means existing launch/generation/cache behaviour is unchanged.
   Pin by unchanged production launcher hashes and model-free CLI opt-in tests.
2. I2: prime only tokens shared by two real template renderings; never synthetic
   user content or a cached answer. Pin prefix/divergence and request-body tests.
3. I3: no ready result on timeout, malformed/absent counters, truncation or incomplete
   inference. Pin deadline/admission/error tests and actual private restart probes.
4. I4: request boundaries preserve session isolation; warmup never changes client
   conversation history. Pin immutable-input tests and foreign-prefix/canary recovery.
5. I5: default model/runtime/voice remain unchanged. Pin provenance fingerprints,
   model SHA and full private browser speech/video checks in Avatar research.

Validate stdlib tests and cache-store existing15/native ownership checks. Test
actual current20B privately; no global/default profile changes. Record findings in
this plan/EXPERIMENT_LOG/anchored summary and Avatar Intel/WBSO report, commit each
completed stage, push VITRIOL feature branch to origin. No employee hours inferred.

## 2026-11-18 14:06 — Implemented, runtime window pending

Commit294e29c adds the opt-in generic tool; original launcher/profile paths are
untouched. `python3 -m unittest libvitriol.test_prompt_prime -q`:9tests,OK.
Ruff and mypy pass. Rust cache-store15 and native ownership checks pass again;
C++ fixed source is byte-identical to the already tested isolated build. Module
and CLI pushed to origin/codex/avatar-startup-integration-20261118. Private Avatar
adapter has6newmodel-free tests and264runtime tests, backend743+5skip/Rust94PASS.
No actual new model or playback result yet. User's keep-live instruction requires
explicit approval for a duplicated full pipeline: approximately18GiBavailable
versus public10GiB plus private avatar/generator copies. A concrete guarded
8m30restore/10minmaximum launcher and recovery tests are prepared; approval
requested and pending. No public service was paused or reconfigured in this task.
No remote Intel message, employeehours or performance improvement claimed.

## 2026-11-18 14:15 — Actual same20B cache and restart checks pass

Bounded private generator CPUQuota400%/Nice10/MemoryHigh8GiB/Max12GiB, public
avatar continuously active. Generic CLI actual native inference admits177tokens;
first answer reuses177. Legacy /slots/0?action=erase returns501 without
--slot-save-path; record this negative, do not silently enable disk caching.
Explicit foreign cache_prompt=false displacement leaves176/177 prefix tokens
recoverable. Re-prime restores full177 reuse; this is NOT a forced RAM-tier
capacity-eviction result. Identical resend,360/403-token history growth, cancellation
and independent next answer pass; synthetic foreign canary absent from answers.
A real private-generator process restart loses all cached prefix: prime reports
cache_n=0/prompt_n=177, next answercache_n=177. Seven recovery chats plus one
post-restart chat and intentional cancellation pass. Source/artifact evidence in
Avatar assets/ditto-live/startup-prime-playback-20261118/component-retry*. Public
worker PIDs/model/voice pins and original20B PID1304035/command unchanged; public
health ready. Brief shared-memory pressure observed; no latency/smoothness claim
from CPU-limited concurrent checks. Full avatar/browser window still awaits user
approval; no live promotion, no home-GPU certification or default profile change.
