// PROVENANCE: original — VITRIOL officina (2026-09-27, parity plan phase 1.4).
// Pure project-scan + MEMORY.md seed builder for the /init command. No I/O
// side effects beyond the caller; everything here is deterministic and
// unit-tested (scan.test.ts).

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

export interface ProjectScan {
  cwd: string;
  name: string;
  /** manifest filenames found at the root (package.json, Cargo.toml, …) */
  buildFiles: string[];
  /** inferred shell commands worth remembering (build/test/run) */
  commands: string[];
  /** language census as "lang (count)" strings, most-filed first */
  languages: string[];
  /** top-level directory names (files excluded), shallow */
  topDirs: string[];
  hasGit: boolean;
  /** orientation docs already present (AGENTS.md, README.md, …) */
  docs: string[];
}

interface Manifest {
  file: string;
  lang: string;
  commands: Array<[string, string]>;
}

const MANIFESTS: Manifest[] = [
  {
    file: "package.json",
    lang: "TypeScript/JavaScript",
    commands: [
      ["build", "npm run build"],
      ["test", "npm test"],
      ["dev", "npm run dev"],
    ],
  },
  { file: "Cargo.toml", lang: "Rust", commands: [["build", "cargo build --release"], ["test", "cargo test"]] },
  { file: "pyproject.toml", lang: "Python", commands: [["test", "python -m pytest"]] },
  { file: "setup.py", lang: "Python", commands: [["test", "python -m pytest"]] },
  { file: "go.mod", lang: "Go", commands: [["build", "go build ./..."], ["test", "go test ./..."]] },
  {
    file: "CMakeLists.txt",
    lang: "C/C++",
    commands: [["configure", "cmake -B build"], ["build", "cmake --build build"]],
  },
  { file: "Makefile", lang: "C/C++ (or make-driven)", commands: [["build", "make"], ["test", "make test"]] },
  { file: "meson.build", lang: "C/C++ (meson)", commands: [["build", "meson compile -C build"]] },
];

const LANG_BY_EXT: Record<string, string> = {
  ".ts": "TypeScript",
  ".tsx": "TypeScript",
  ".js": "JavaScript",
  ".mjs": "JavaScript",
  ".jsx": "JavaScript",
  ".py": "Python",
  ".rs": "Rust",
  ".go": "Go",
  ".c": "C",
  ".h": "C",
  ".cpp": "C++",
  ".cc": "C++",
  ".hpp": "C++",
  ".cu": "CUDA",
  ".java": "Java",
  ".sh": "Shell",
  ".lua": "Lua",
};

const SHALLOW_SCAN_CAP = 400;

export function scanProject(cwd: string): ProjectScan {
  const name = cwd.slice(cwd.lastIndexOf("/") + 1) || cwd;
  const buildFiles = MANIFESTS.filter((m) => existsSync(join(cwd, m.file))).map((m) => m.file);
  const commands: string[] = [];
  for (const m of MANIFESTS) {
    if (!buildFiles.includes(m.file)) continue;
    for (const [label, cmd] of m.commands) {
      if (label === "test" || label === "build" || label === "dev") {
        // package.json scripts only suggested when the script exists
        if (m.file === "package.json" && !scriptExists(cwd, label)) continue;
        commands.push(cmd);
      }
    }
  }

  let topDirs: string[] = [];
  let hasGit = false;
  const docs: string[] = [];
  const counts = new Map<string, number>();
  let seen = 0;

  let entries: string[] = [];
  try {
    entries = readdirSync(cwd);
  } catch {
    entries = [];
  }
  for (const e of entries) {
    const p = join(cwd, e);
    let st;
    try {
      st = statSync(p);
    } catch {
      continue;
    }
    if (st.isDirectory()) {
      if (e === ".git") hasGit = true;
      else if (!e.startsWith(".")) topDirs.push(e);
      continue;
    }
    if (/^(AGENTS|CLAUDE|README|GEMINI)\.md$/i.test(e)) docs.push(e);
    const dot = e.lastIndexOf(".");
    if (dot > 0) {
      const lang = LANG_BY_EXT[e.slice(dot)];
      if (lang) counts.set(lang, (counts.get(lang) ?? 0) + 1);
    }
  }
  topDirs.sort();

  // one shallow level into top dirs for a slightly honest census
  for (const d of topDirs.slice(0, 12)) {
    let sub: string[] = [];
    try {
      sub = readdirSync(join(cwd, d));
    } catch {
      continue;
    }
    for (const e of sub) {
      if (seen >= SHALLOW_SCAN_CAP) break;
      const dot = e.lastIndexOf(".");
      if (dot <= 0) continue;
      const lang = LANG_BY_EXT[e.slice(dot)];
      if (lang) {
        counts.set(lang, (counts.get(lang) ?? 0) + 1);
        seen++;
      }
    }
  }

  const languages = [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8)
    .map(([lang, n]) => `${lang} (${n})`);

  return { cwd, name, buildFiles, commands, languages, topDirs, hasGit, docs };
}

function scriptExists(cwd: string, script: string): boolean {
  try {
    const pkg = JSON.parse(readFileSync(join(cwd, "package.json"), "utf8")) as {
      scripts?: Record<string, unknown>;
    };
    return Boolean(pkg.scripts?.[script]);
  } catch {
    return false;
  }
}

/** Render the seed document. Handled by init/index.ts's write. */
export function buildMemorySeed(scan: ProjectScan, timestamp: string): string {
  const lines: string[] = [
    `# ${scan.name} — project memory`,
    "",
    `Seeded by /init ${timestamp} (deterministic scan; correct anything wrong inline).`,
    "",
    "## Shape",
    `- Languages: ${scan.languages.length ? scan.languages.join(", ") : "none detected"}`,
    `- Top-level dirs: ${scan.topDirs.length ? scan.topDirs.join(", ") : "none"}`,
    `- Git: ${scan.hasGit ? "yes" : "no"}`,
    scan.docs.length ? `- Orientation docs: ${scan.docs.join(", ")}` : "- Orientation docs: none",
    "",
    "## Toolchain",
    scan.buildFiles.length ? `- Manifests: ${scan.buildFiles.join(", ")}` : "- Manifests: none detected",
    scan.commands.length ? `- Commands:\n${scan.commands.map((c) => `  - \`${c}\``).join("\n")}` : "- Commands: unknown — fill in",
    "",
    "## Conventions",
    "- (fill in: style guide, review rules, forbidden moves)",
    "",
  ];
  return lines.join("\n");
}
