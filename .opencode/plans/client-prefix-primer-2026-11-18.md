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
