# Permission visibility — plan (2026-11-21)

## Owner report
1. The permission prompt doesn't show the command being run — no way to judge
   safety.
2. No way to view the active permission rules anywhere.

## Findings (traced)

### 1. The prompt DOES carry the command — for bash only, and possibly not
where you look.

`permissions-guard/index.ts`:
- **bash/powershell**: `buildAskMessage` (line ~209) includes the FULL command
  (up to 2000 chars, multi-line preserved, `$ ` prefix, plus the mode).
  This was fixed 2026-11-17 precisely because it used to be collapsed to one
  line and cut at 120 chars.
- **write/edit**: the dialog detail is `${tool} ${path}` — the PATH only.
  The file's CONTENT (what will be written) is never shown. For a `write`
  to an outside-cwd path, the operator approves a path with no idea what is
  entering their disk. That is the real safety gap.

So: if your memory of "no command shown" comes from a **write/edit prompt**,
that's by construction — and the fix is to include a bounded content preview
in the dialog. If it came from a **bash** prompt, the more likely cause is the
TUI's dialog rendering: `ctx.ui.confirm(title, message)` renders `message`
through pi's dialog; a 2000-char multi-line body may be scrolled/clipped by
the widget, hiding the command line. Needs a rendering check in the TUI
(question.rs / dialogs) before assuming the message is missing.

### 2. Permission rules live in THREE places with no viewer:
- `~/.config/trismegistus/permissions.json` — generated mirror (path rules,
  tool+pattern+action), rewritten by `perms-sync` (source_hash).
- `~/.vitriol/officina/perms/<slug>-<hash8>.json` — project allowlists
  (command/program rules, mode 0600, never in the repo).
- `~/.vitriol/officina/permissions-mode` — the active mode (ask/auto/yolo).
Nothing surfaces them: no `/permissions` command, no doctor check, no dialog
showing which rule matched (the dialog shows only the verdict reason, e.g.
"rule src/** requires approval" — actually that names it; but there is no way
to LIST what is stored).

## Stages

**S1 — content preview in write/edit dialogs.**
`buildAskMessage` gains the file content preview for write/edit: first ~40
lines / 2000 chars of `input.content`, prefixed `+ ` per line, capped with a
"… (N more lines)" tail. Edit shows the same bounded preview of the new
content. Read tool: path only (unchanged).

**S2 — `/permissions` command.**
Registers in permissions-guard: renders a compact report into a dialog:
- active mode + source file
- global path rules (from permissions.json)
- this project's allowlist rules (command/program patterns)
- protected roots summary (already hardcoded in policy.ts — listed as
  "always ask")
Pure renderer in `perms-report.ts` (testable), thin command wrapper.

**S3 — dialog rendering audit.** Verify the TUI dialog scrolls/clips long
bodies legibly (question.rs); if it clips at N lines, cap S1 previews to what
fits rather than what we send.

## Invariants

- **I1**: the ask dialog for bash ALWAYS contains the full command (existing
  2026-11-17 behaviour, now pinned by a test).
- **I2**: the ask dialog for write/edit contains a bounded preview of the
  content being written (new). Pinned by unit test on buildAskMessage.
- **I3**: previews are capped — a 1 MB write must not hang a frame (cap 2000
  chars / 40 lines, tail summary line).
- **I4**: `/permissions` never truncates rule patterns (patterns ARE the
  safety information) and shows the source file for each rule set.
- **I5**: no new permission state is created — the command is read-only.
- **I6**: protected roots remain un-bypassable (unchanged).

## Named tests
- `permissions-guard.test.ts`: `ask_message_contains_full_command` (I1),
  `write_dialog_shows_content_preview` (I2), `preview_capped_on_huge_content`
  (I3).
- `perms-report.test.ts` (new): `lists_mode_global_and_project_rules` (I4),
  `read_only_no_store_writes` (I5), `protected_roots_listed`.
