## Summary

<!-- What changed and why. Link the issue/report if one exists. -->

## Type

- [ ] feat
- [ ] fix
- [ ] docs
- [ ] chore / ci / style
- [ ] engine patch (PROVENANCE note required)

## Test plan

<!-- Exact commands and results. Delete lines that do not apply. -->

- [ ] `cargo fmt --check` + `cargo test --locked` (affected crates)
- [ ] `cd officina && npm test && npm run typecheck`
- [ ] `shellcheck --severity=error scripts/*.sh`
- [ ] engine smoke (if engine/ or launch flags changed)
- [ ] fingerprint diff recorded (if speed-bearing flags changed)

## Checklist

- [ ] No secrets, models, slot state, or personal paths committed
- [ ] Docs updated for user-visible changes
- [ ] One logical change per commit
