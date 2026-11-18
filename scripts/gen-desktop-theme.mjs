#!/usr/bin/env node
// Generate a desktop theme package from the ONE palette source of truth.
//
// The palette lives in `officina/theme/palette.json` — the same file the Officina
// chat TUI, the vitriol-tui dashboard and the Vitriolum JS palette are pinned to.
// This script turns it into the surfaces a desktop needs: VSCode, Plasma 6,
// kitty, alacritty, zellij and termux.
//
// Why generate instead of hand-maintaining: every surface used to be its own copy
// of the palette, and copies drift (the dashboard shipped the old warning orange
// for four days). One source, N outputs, zero drift.
//
// It NEVER applies anything. Generating writes files under --out and stops; making
// them live is a separate, deliberate act (`install.sh` / `apply.sh`, or your
// desktop settings). The owner's active desktop stays whatever it is.
//
// Usage:
//   node scripts/gen-desktop-theme.mjs                    # vitriol → themes/vitriol
//   node scripts/gen-desktop-theme.mjs --palette aether
//   node scripts/gen-desktop-theme.mjs --out /tmp/theme-check --dry-run
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..");
const PALETTE_FILE = join(repoRoot, "officina", "theme", "palette.json");

const argv = process.argv.slice(2);
const argOf = (flag, fallback) => {
  const i = argv.indexOf(flag);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
};
const DRY = argv.includes("--dry-run");
const PALETTE = argOf("--palette", "vitriol").toLowerCase();
const THEME_NAMES = {
  vitriol: { title: "VITRIOL", id: "vitriol", dir: "vitriol" },
  aether: { title: "Aether", id: "aether", dir: "aether" },
  alkahest: { title: "Alkahest", id: "alkahest", dir: "alkahest" },
};
const meta = THEME_NAMES[PALETTE];
if (!meta) {
  console.error(`unknown palette '${PALETTE}' — try one of ${Object.keys(THEME_NAMES).join(", ")}`);
  process.exit(2);
}
const OUT = resolve(argOf("--out", join(homedir(), "Desktop", "Projects", "themes", meta.dir)));

const doc = JSON.parse(readFileSync(PALETTE_FILE, "utf8"));
const p = doc.palettes[PALETTE];
if (!p) {
  console.error(`palette.json has no '${PALETTE}'`);
  process.exit(2);
}

/** `#rrggbb` → `{r,g,b}` (0-255). */
function rgb(hex) {
  const h = hex.replace("#", "");
  return { r: parseInt(h.slice(0, 2), 16), g: parseInt(h.slice(2, 4), 16), b: parseInt(h.slice(4, 6), 16) };
}
const triple = (hex) => {
  const { r, g, b } = rgb(hex);
  return `${r},${g},${b}`;
};
const up = (hex) => hex.toUpperCase();

/**
 * The 16 ANSI slots, derived from the roles the TUI already uses so terminal
 * output matches the panels around it. Black/white come from the grounds and the
 * text role rather than from literal greys.
 */
function ansi() {
  return {
    black: p.border_dim,
    red: p.red,
    green: p.green,
    yellow: p.gold,
    blue: p.cold_blue,
    magenta: p.violet,
    cyan: p.cyan,
    white: p.muted,
    brightBlack: p.faint,
    brightRed: p.red,
    brightGreen: p.green,
    brightYellow: p.light_yellow,
    brightBlue: p.silver,
    brightMagenta: p.violet,
    brightCyan: p.cyan,
    brightWhite: p.text,
  };
}

// ── VSCode ────────────────────────────────────────────────────────────────
function vscodeTheme() {
  const a = ansi();
  const colors = {
    "editor.background": up(p.bg),
    "editor.foreground": up(p.text),
    "editorCursor.foreground": up(p.gold),
    "editor.selectionBackground": up(p.border_dim),
    "editor.lineHighlightBackground": up(p.panel),
    "editorLineNumber.foreground": up(p.faint),
    "editorLineNumber.activeForeground": up(p.muted),
    "editorIndentGuide.background1": up(p.border_dim),
    "editorWhitespace.foreground": up(p.border_dim),
    "editorGutter.background": up(p.bg),
    "editorError.foreground": up(p.red),
    "editorWarning.foreground": up(p.orange),
    "editorInfo.foreground": up(p.cyan),
    "editorBracketMatch.border": up(p.gold),
    "activityBar.background": up(p.panel),
    "activityBar.foreground": up(p.text),
    "activityBarBadge.background": up(p.gold),
    "activityBarBadge.foreground": up(p.bg),
    "sideBar.background": up(p.panel),
    "sideBar.foreground": up(p.text),
    "sideBar.border": up(p.border_dim),
    "sideBarSectionHeader.background": up(p.bg),
    "statusBar.background": up(p.panel),
    "statusBar.foreground": up(p.text),
    "statusBar.border": up(p.border_dim),
    "titleBar.activeBackground": up(p.panel),
    "titleBar.activeForeground": up(p.text),
    "panel.background": up(p.panel),
    "panel.border": up(p.border_dim),
    "terminal.background": up(p.bg),
    "terminal.foreground": up(p.text),
    "terminal.ansiBlack": up(a.black),
    "terminal.ansiRed": up(a.red),
    "terminal.ansiGreen": up(a.green),
    "terminal.ansiYellow": up(a.yellow),
    "terminal.ansiBlue": up(a.blue),
    "terminal.ansiMagenta": up(a.magenta),
    "terminal.ansiCyan": up(a.cyan),
    "terminal.ansiWhite": up(a.white),
    "terminal.ansiBrightBlack": up(a.brightBlack),
    "terminal.ansiBrightRed": up(a.brightRed),
    "terminal.ansiBrightGreen": up(a.brightGreen),
    "terminal.ansiBrightYellow": up(a.brightYellow),
    "terminal.ansiBrightBlue": up(a.brightBlue),
    "terminal.ansiBrightMagenta": up(a.brightMagenta),
    "terminal.ansiBrightCyan": up(a.brightCyan),
    "terminal.ansiBrightWhite": up(a.brightWhite),
    "button.background": up(p.border_dim),
    "button.foreground": up(p.text),
    "input.background": up(p.bg),
    "input.border": up(p.border_dim),
    "dropdown.background": up(p.panel),
    "list.activeSelectionBackground": up(p.border_dim),
    "list.hoverBackground": up(p.panel),
    "badge.background": up(p.gold),
    "badge.foreground": up(p.bg),
    "focusBorder": up(p.gold),
    "scrollbarSlider.background": up(p.border_dim) + "80",
    "gitDecoration.addedResourceForeground": up(p.green),
    "gitDecoration.modifiedResourceForeground": up(p.orange),
    "gitDecoration.deletedResourceForeground": up(p.red),
  };
  const tokenColors = [
    { scope: ["comment", "punctuation.definition.comment"], settings: { foreground: up(p.faint), fontStyle: "italic" } },
    { scope: ["string", "string.quoted"], settings: { foreground: up(p.green) } },
    { scope: ["constant.numeric"], settings: { foreground: up(p.violet) } },
    { scope: ["constant.language", "constant.character.escape"], settings: { foreground: up(p.orange) } },
    { scope: ["keyword", "storage"], settings: { foreground: up(p.gold) } },
    { scope: ["keyword.operator"], settings: { foreground: up(p.muted) } },
    { scope: ["entity.name.function", "support.function"], settings: { foreground: up(p.cyan) } },
    { scope: ["entity.name.type", "support.type", "entity.name.class"], settings: { foreground: up(p.silver) } },
    { scope: ["variable", "variable.other"], settings: { foreground: up(p.text) } },
    { scope: ["entity.name.tag"], settings: { foreground: up(p.red) } },
    { scope: ["meta.preprocessor", "entity.name.namespace"], settings: { foreground: up(p.orange) } },
    { scope: ["invalid"], settings: { foreground: up(p.red) } },
    { scope: ["markup.heading"], settings: { foreground: up(p.gold), fontStyle: "bold" } },
    { scope: ["markup.bold"], settings: { fontStyle: "bold" } },
    { scope: ["markup.italic"], settings: { fontStyle: "italic" } },
  ];
  return {
    name: `${meta.title} (Officina)`,
    type: "dark",
    colors,
    tokenColors,
  };
}

function vscodePackage() {
  const id = `officina-${meta.id}`;
  return {
    name: id,
    displayName: `${meta.title} — Officina`,
    description: `${meta.title} palette for Officina / VITRIOL — generated from officina/theme/palette.json`,
    version: "0.1.0",
    publisher: "randozart",
    license: "Apache-2.0 OR MIT",
    engines: { vscode: "^1.70.0" },
    categories: ["Themes"],
    contributes: {
      themes: [{ label: `${meta.title} (Officina)`, uiTheme: "vs-dark", path: `./themes/${id}-color-theme.json` }],
    },
  };
}

// ── Plasma 6 ──────────────────────────────────────────────────────────────
function plasmaColors() {
  const sec = (name, bg, fg, extra = []) =>
    [
      `[Colors:${name}]`,
      `BackgroundNormal=${triple(bg)}`,
      `BackgroundAlternate=${triple(p.panel)}`,
      `ForegroundNormal=${triple(fg)}`,
      `ForegroundInactive=${triple(p.muted)}`,
      `ForegroundActive=${triple(p.gold)}`,
      `ForegroundLink=${triple(p.cyan)}`,
      `ForegroundVisited=${triple(p.violet)}`,
      `ForegroundNegative=${triple(p.red)}`,
      `ForegroundNeutral=${triple(p.orange)}`,
      `ForegroundPositive=${triple(p.green)}`,
      `DecorationFocus=${triple(p.gold)}`,
      `DecorationHover=${triple(p.cyan)}`,
      ...extra,
      "",
    ].join("\n");
  return [
    `# ${meta.title} — generated from officina/theme/palette.json`,
    "[General]",
    `Name=${meta.title}`,
    "ColorScheme=Dark",
    "shadeSortColumn=true",
    "",
    sec("Window", p.panel, p.text),
    sec("View", p.bg, p.text),
    sec("Button", p.border_dim, p.text),
    sec("Selection", p.border_dim, p.gold),
    sec("Tooltip", p.panel, p.text),
    sec("Complementary", p.bg, p.text),
    "[WM]",
    `activeBackground=${triple(p.panel)}`,
    `activeForeground=${triple(p.text)}`,
    `inactiveBackground=${triple(p.bg)}`,
    `inactiveForeground=${triple(p.muted)}`,
    "",
  ].join("\n");
}

// ── Terminals ─────────────────────────────────────────────────────────────
function kitty() {
  const a = ansi();
  const lines = [
    `# ${meta.title} — generated from officina/theme/palette.json`,
    `background ${up(p.bg)}`,
    `foreground ${up(p.text)}`,
    `cursor ${up(p.gold)}`,
    `selection_background ${up(p.border_dim)}`,
    `selection_foreground ${up(p.text)}`,
  ];
  const order = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
  order.forEach((name, i) => lines.push(`color${i} ${up(a[name])}`));
  order.forEach((name, i) => lines.push(`color${i + 8} ${up(a["bright" + name[0].toUpperCase() + name.slice(1)])}`));
  return lines.join("\n") + "\n";
}

function alacritty() {
  const a = ansi();
  return `# ${meta.title} — generated from officina/theme/palette.json
[colors.primary]
background = "${up(p.bg)}"
foreground = "${up(p.text)}"

[colors.cursor]
text = "${up(p.bg)}"
cursor = "${up(p.gold)}"

[colors.selection]
text = "${up(p.text)}"
background = "${up(p.border_dim)}"

[colors.normal]
black = "${up(a.black)}"
red = "${up(a.red)}"
green = "${up(a.green)}"
yellow = "${up(a.yellow)}"
blue = "${up(a.blue)}"
magenta = "${up(a.magenta)}"
cyan = "${up(a.cyan)}"
white = "${up(a.white)}"

[colors.bright]
black = "${up(a.brightBlack)}"
red = "${up(a.brightRed)}"
green = "${up(a.brightGreen)}"
yellow = "${up(a.brightYellow)}"
blue = "${up(a.brightBlue)}"
magenta = "${up(a.brightMagenta)}"
cyan = "${up(a.brightCyan)}"
white = "${up(a.brightWhite)}"
`;
}

function zellij() {
  const a = ansi();
  const c = (hex) => up(hex);
  return `// ${meta.title} — generated from officina/theme/palette.json
// Drop into config.kdl:  themes { ${meta.id} { ... } }  then set theme "${meta.id}"
themes {
    ${meta.id} {
        fg "${c(p.text)}"
        bg "${c(p.bg)}"
        black "${c(a.black)}"
        red "${c(a.red)}"
        green "${c(a.green)}"
        yellow "${c(a.yellow)}"
        blue "${c(a.blue)}"
        magenta "${c(a.magenta)}"
        cyan "${c(a.cyan)}"
        white "${c(a.white)}"
        orange "${c(p.orange)}"
    }
}
`;
}

function termux() {
  const a = ansi();
  const order = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
  const lines = [`# ${meta.title} — generated from officina/theme/palette.json`];
  order.forEach((name, i) => lines.push(`color${i}=${up(a[name])}`));
  order.forEach((name, i) => lines.push(`color${i + 8}=${up(a["bright" + name[0].toUpperCase() + name.slice(1)])}`));
  lines.push(`background=${up(p.bg)}`, `foreground=${up(p.text)}`, `cursor=${up(p.gold)}`);
  return lines.join("\n") + "\n";
}

function applyScript() {
  const scheme = `${meta.title.replace(/\s+/g, "")}Dark`;
  return `#!/usr/bin/env bash
# ${meta.title} — apply the generated theme to this machine.
#
# Nothing here runs automatically: generating a theme writes files only. This
# script installs them for the current user and prints what it changed.
set -euo pipefail
HERE="$(cd "$(dirname "\${BASH_SOURCE[0]}")" && pwd)"

echo "[${meta.id}] VSCode: copy $HERE/vscode/officina-${meta.id} into ~/.vscode/extensions/"
mkdir -p "$HOME/.vscode/extensions"
rm -rf "$HOME/.vscode/extensions/officina-${meta.id}"
cp -r "$HERE/vscode/officina-${meta.id}" "$HOME/.vscode/extensions/"
echo "        then: Preferences: Color Theme -> ${meta.title} (Officina)"

echo "[${meta.id}] Plasma: install + apply ${scheme}"
mkdir -p "$HOME/.local/share/color-schemes"
cp "$HERE/plasma/${scheme}.colors" "$HOME/.local/share/color-schemes/"
command -v plasma-apply-colorscheme >/dev/null && plasma-apply-colorscheme "${scheme}" || echo "        (plasma-apply-colorscheme not found; pick it in System Settings)"

echo "[${meta.id}] terminals: configs are in $HERE/terminal/ — add the include/import yourself"
echo "        kitty:     include ${meta.id}.conf"
echo "        alacritty: add ${meta.id}.toml to your import list"
echo "        zellij:    paste the themes block into config.kdl and set theme \"${meta.id}\""
echo "[${meta.id}] done"
`;
}

function readme() {
  return `# ${meta.title}

Desktop theme generated from \`officina/theme/palette.json\` — the single source of
truth shared with the Officina chat TUI, the \`vitriol-tui\` dashboard and the
Vitriolum extension palette.

**Regenerate** (never hand-edit):

\`\`\`bash
node <repo>/scripts/gen-desktop-theme.mjs --palette ${PALETTE} --out ${OUT}
\`\`\`

## Contents

| path | what |
|---|---|
| \`vscode/officina-${meta.id}/\` | VSCode colour theme extension |
| \`plasma/${meta.title.replace(/\\s+/g, "")}Dark.colors\` | KDE Plasma 6 colour scheme |
| \`terminal/kitty/${meta.id}.conf\` | kitty |
| \`terminal/alacritty/${meta.id}.toml\` | alacritty |
| \`terminal/zellij/${meta.id}.kdl\` | zellij theme block |
| \`terminal/termux/colors.properties\` | termux |

## Apply (deliberate, this folder does not do it for you)

- VSCode: copy \`vscode/officina-${meta.id}\` into \`~/.vscode/extensions/\`, reload, then
  *Preferences: Color Theme* → **${meta.title} (Officina)**.
- Plasma: \`plasma-apply-colorscheme ${meta.title.replace(/\\s+/g, "")}Dark\` (after copying the
  \`.colors\` file into \`~/.local/share/color-schemes/\`).
- kitty: add \`include ${meta.id}.conf\` to \`kitty.conf\`.
- alacritty: reference it from \`alacritty.toml\`'s import list.
- zellij: paste the \`themes { … }\` block into \`config.kdl\`.

Palette: ${Object.entries(p)
    .filter(([k]) => k !== "fire" && k !== "syntax")
    .map(([k, v]) => `${k} \`${v}\``)
    .join(" · ")}
`;
}

const FILES = {
  [`vscode/officina-${meta.id}/package.json`]: JSON.stringify(vscodePackage(), null, 2) + "\n",
  [`vscode/officina-${meta.id}/themes/officina-${meta.id}-color-theme.json`]:
    JSON.stringify(vscodeTheme(), null, 2) + "\n",
  [`plasma/${meta.title.replace(/\s+/g, "")}Dark.colors`]: plasmaColors(),
  [`terminal/kitty/${meta.id}.conf`]: kitty(),
  [`terminal/alacritty/${meta.id}.toml`]: alacritty(),
  [`terminal/zellij/${meta.id}.kdl`]: zellij(),
  [`terminal/termux/colors.properties`]: termux(),
  ["README.md"]: readme(),
  // Apply is deliberately an explicit script: generating never touches a live
  // desktop, and running this is a conscious act (the owner keeps Aetherskiff).
  ["apply.sh"]: applyScript(),
  [".generated-from"]: `${PALETTE_FILE}\nversion=${doc.version}\n`,
};

for (const [rel, body] of Object.entries(FILES)) {
  const path = join(OUT, rel);
  if (DRY) {
    console.log(`would write ${path} (${body.length} B)`);
    continue;
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, body);
  console.log(`wrote ${path}`);
}

console.log(
  DRY
    ? `dry run: ${Object.keys(FILES).length} files for '${PALETTE}' would land in ${OUT}`
    : `${meta.title}: ${Object.keys(FILES).length} files in ${OUT} (nothing applied)`,
);
