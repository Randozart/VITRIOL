# Permission modal: show the real command, and add "always for this project"

**Date:** 2026-11-17 19:50
**Mode:** PLAN (research only — nothing changed)
**Owner:** "the permission modal should show the triggering command, and an allow always for that
project."

## 1. What the modal shows today

`officina/.pi/extensions/permissions-guard/index.ts:116`

```ts
const yes = await ctx.ui?.confirm(
  "permissions",
  `Allow ${tool}?\n${verdict.reason}\n${detail}\n(mode: ${mode})`,
);
```

For bash, `detail` is `shortCommand(command)` (`index.ts:224`):

```ts
const one = command.replace(/\s+/g, " ").trim();
return one.length > 120 ? `${one.slice(0, 119)}…` : one;
```

So the dialog *does* carry the command, but:

1. **it is collapsed to one line and cut at 120 chars** — a heredoc, a `&&` chain or a `python3 -`
   block is unreadable exactly when it matters most;
2. the TUI renders it as a **plain message with no emphasis on the command** (title `permissions`,
   then reason, then command, then mode), so there is nothing that says "this is what you are
   approving";
3. it is a **Confirm (Yes/No)**: there is no middle choice, so a recurring, individually-harmless
   command has to be approved by hand every single time.

For `edit`/`write`/`read` the detail is `"<tool> <path>"` — fine, but the same one-line limit applies
to long paths.

## 2. Where "always" would have to live

The policy is **global and generated**: `~/.config/trismegistus/permissions.json`
(`default_action`, `rules[{tool, pattern, action}]`, `source_hash`) written by
`trismegistus perms-sync`. Writing a project rule *into that file* would be destroyed by the next
sync (it is a mirror with a source hash), and there is **no project-scoped store at all**.

So "allow always for this project" needs a new store. Two shapes:

| option | where | verdict |
|---|---|---|
| **A. in-repo** `<project>/.officina/permissions.allow.json` | visible, reviewable, shareable with the project | pollutes the repo; can be committed by accident; a repo can *ship* allow rules |
| **B. per-project, under the operator's home** `~/.vitriol/officina/perms/<slug>-<hash>.json` | repo stays clean, survives `perms-sync`, never shipped | invisible unless `/perms` shows it |

**Recommendation: B**, with `/perms` (and `/perms clear <n>`) so it is inspectable. Rationale: this
repo publishes snapshots of `officina/`; a policy file that grants agent permissions must never be
part of that snapshot by accident.

Rule shape (deliberately narrow):

```json
{"version": 1, "project": "/home/randozart/Desktop/Projects/VITRIOL",
 "rules": [{"tool": "bash", "kind": "command", "pattern": "make -j12", "at": "2026-11-17T19:50"},
           {"tool": "bash", "kind": "program", "pattern": "cargo", "at": "..."}]}
```

- `kind: "command"` matches the **exact** normalised command (whitespace-collapsed) — the safe
  default;
- `kind: "program"` matches the leading program only (`cargo`, `git`) — opt-in, since that is much
  broader (`git push --force` included).

## 3. Plan

### Stage 1 — the modal shows the command honestly
- `permissions-guard`: send the **full** command (no 120-char cut) as part of the message, and keep a
  shortened form only for the one-line notice/log line. The dialog already renders
  `message.lines()` with wrapping and a `body.height - 2` limit, so long commands fit; add
  `(+N lines)` when it had to clip.
- TUI: render the **command block on its own tinted line(s)** (the panel's dim surface) so it reads
  as "the thing being approved", distinct from the reason sentence.
- Caps: hard-limit what a *dialog* carries (say 2 000 chars) so a pathological command cannot hang
  the frame; the ledger keeps the full text.

### Stage 2 — three-way choice instead of yes/no
Replace `confirm` with `select`:

```
Allow bash?                       [title]
reason: risky shell               [message]
  $ make -j12 && ./target/release/officina --help
  (mode: auto)
▸ Allow once
  Allow this command in this project
  Allow all cargo in this project      (only when a program is detectable)
  Deny
```

- The TUI's `DialogKind::Select` currently renders **only the options** (`layout.rs:1367`); it must
  also render `dialog.message` above them, exactly as Confirm does (`layout.rs:1395`). That is the
  small render change this needs.
- Keyboard: `1..4` pick directly, `↑↓/enter` as today, `esc` = deny (fail closed).

### Stage 3 — persist and honour the project allowlist
- New module `perms-project.ts` in permissions-guard: `loadProjectRules(cwd)`, `addProjectRule()`
  (atomic write, 0600), `matchCommand()` / `matchProgram()`.
- Consult it **after** protected roots and explicit denies, **before** the ask: a protected root can
  never be allowlisted by a project rule (invariant below).
- Emit `lc-perms/project-allow` events so the decision is auditable.

### Invariants

- **I1 (fail closed):** an unreadable/corrupt project-allow file means *no* rules — never allow-all.
- **I2 (protected roots win):** a project rule can never allow a path inside `protectedGlobs()`
  (`/etc/**`, `~/.ssh/**`, `~/.vitriol/secrets`, `**/.env`, …). Test: an `allow` rule for
  `~/.vitriol/secrets` is ignored and still asks.
- **I3 (exact by default):** `kind: "command"` matches the normalised command only; no prefix,
  glob or substring matching. Test: allowing `make -j12` does not allow `make -j12 && rm -rf /`.
- **I4 (project scope):** a rule recorded for project A never applies in project B. Test with two
  tmp project paths.
- **I5 (parity):** with no project-allow file, behaviour is byte-identical to today (one Confirm).
- **I6 (no repo writes):** the store lives under `$HOME`, never in the repo; a test asserts no file
  is created inside `cwd`.

## 4. Files

- `.pi/extensions/permissions-guard/index.ts` — message shape, select instead of confirm, consult
  the store.
- `.pi/extensions/permissions-guard/perms-project.ts` (new) + tests.
- `.pi/extensions/permissions-guard/policy.ts` — export the containment helper for I2.
- `officina/tui/src/tui/layout.rs` — render `message` for `DialogKind::Select`; command emphasis.
- `officina/tui/src/tui/mod.rs` — digit keys for select; `esc` → deny.
