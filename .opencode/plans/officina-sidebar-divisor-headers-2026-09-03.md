# Sidebar divisor headers + Plans restyle

**Date:** 2026-09-03 08:00 UTC
**Scope:** `officina/.pi/extensions/session-panel/index.ts` only
(hand-maintained extension — no build-patch/canary concern). The Rust TUI
renders sidebar widget lines verbatim (`ansi::parse_line`), so zero Rust
changes. Discovered via explore pass: no "plans" string exists anywhere —
the owner's "plan menu" is the sidebar `tasks` section (index.ts:200-216,
label `"tasks "` at line 209) with the checklist counts glued onto the
label row.

---

## 1. Embedded-header divider helper

`thickDiv`/`thinDiv` (lines 79-80) → one helper, exported for tests:

```ts
export const embDiv = (name: string): string => {
    const label = ` ${name} `;
    const lead = "──";
    const rest = CONTENT_W - visibleLen(lead) - visibleLen(label);
    return sc(MUTED, lead + label + "─".repeat(rest));
};
```

Renders `── Plans ────────────────────` — name at the START of the rule
(owner phrasing: "each divisor start with their header name inside of the
divisor"), padded to exactly CONTENT_W (40) visible columns.

## 2. Divider call sites → group names

| section | line | before | after |
|---|---|---|---|
| div1 | 146-149 | bare rule | `embDiv("Engine")` |
| div2 | 191-194 | bare rule | `embDiv("Plans")` |
| div3 | 250-253 | bare rule | `embDiv("Session")` |
| div4 | 282-285 | bare rule | `embDiv("Commands")` |

Group content after each: div1 → ctx/ing/eng stats; div2 → tasks
checklist/scratchpad/files; div3 → session/skills/knowledge; div4 → hints.
Sub-rows keep their muted inline labels (`ctx `, `note `, `files: `) as
sub-items within the named group.

## 3. Plans section restyle (lines 200-216)

- Line 209: drop the inline `tasks ` prefix — the counts row is bare:
  `[>] 1 · [ ] 3 · 5 done`
- The NAME lives in the divider (div2 → `── Plans ─…`)
- Item rows (211-214) unchanged (2-space indent, `[>]`/`[ ]` marks)

## 4. Tests

New `officina/.pi/extensions/session-panel/session-panel.test.ts`:
- `embDiv("Plans")` visible length (ANSI-stripped via visibleLen) === 40
- format: starts `── `, contains ` Plans `, ends with `─` run
- ansi codes present (MUTED sc wrapping)

Run: `npx vitest --run .pi/extensions` + `npm run typecheck`.

## Commit

| Commit | Notes |
|---|---|
| `d9c0635` sidebar divisor headers — Plans group restyle | 519 vitest green, tsc clean |

## Outcome notes

- sc() = `color + txt + RESET` — escape PRECEDES text, so format
  assertions must run on the ANSI-stripped string (first test draft
  called `.startsWith` on the raw styled line and failed correctly).
- `CONTENT_W` / `visibleLen` / `embDiv` exported at module level for
  tests; importing the module is side-effect-free (the default export's
  body holds all registration logic).
- The extension is source-live (no rebuild needed) — takes effect on the
  next officina launch; no binary reinstall required for this change.
