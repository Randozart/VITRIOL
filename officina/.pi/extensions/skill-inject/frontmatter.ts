// Minimal YAML frontmatter parser — enough for the fields little-coder uses.
// Mirrors skill/loader.py::_parse_skill_file's behavior, no external deps.

export interface Frontmatter {
  [key: string]: string | string[] | number | boolean | undefined;
}

export interface ParsedSkill {
  frontmatter: Frontmatter;
  body: string;
}

export function parseSkillFile(text: string): ParsedSkill | null {
  const parts = text.split("---");
  if (parts.length < 3) return null;
  const fmText = parts[1].trim();
  const body = parts.slice(2).join("---").trim();
  const fm: Frontmatter = {};
  for (const line of fmText.split("\n")) {
    const m = line.match(/^(\w[\w_-]*)\s*:\s*(.*)$/);
    if (!m) continue;
    const key = m[1].trim();
    let val = m[2].trim();
    if (val.startsWith("[") && val.endsWith("]")) {
      fm[key] = val
        .slice(1, -1)
        .split(",")
        .map((s) => s.trim().replace(/^["']|["']$/g, ""))
        .filter((s) => s.length > 0);
    } else if (/^-?\d+$/.test(val)) {
      fm[key] = parseInt(val, 10);
    } else if (val === "true" || val === "false") {
      fm[key] = val === "true";
    } else {
      fm[key] = val;
    }
  }
  return { frontmatter: fm, body };
}
