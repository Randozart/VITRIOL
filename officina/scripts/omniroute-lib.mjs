// Pure env-file helpers for the OmniRoute bootstrap. Separated so tests do not
// import the CLI entrypoint (which runs main()).

export function parseEnvFile(content) {
  const values = new Map();
  for (const rawLine of String(content).split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    values.set(key, value);
  }
  return values;
}

export function formatEnvValue(value) {
  const text = String(value);
  return /^[A-Za-z0-9_./:@%+,=-]+$/.test(text) ? text : JSON.stringify(text);
}

export function upsertEnvFile(content, updates) {
  const lines = String(content).split(/\r?\n/);
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  const seen = new Set();
  const out = lines.map((line) => {
    const match = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=/);
    if (!match) return line;
    const key = match[1];
    if (!(key in updates)) return line;
    seen.add(key);
    return `${key}=${formatEnvValue(updates[key])}`;
  });
  for (const [key, value] of Object.entries(updates)) {
    if (!seen.has(key)) out.push(`${key}=${formatEnvValue(value)}`);
  }
  while (out.length > 0 && out[out.length - 1] === "") out.pop();
  return `${out.join("\n")}\n`;
}


export function systemdUnit({ bin, envFile, port = 20128 }) {
  return `[Unit]
Description=OmniRoute local gateway for Officina
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${bin} serve --headless --port ${port}
Environment=PORT=${port}
Environment=OMNIROUTE_HEADLESS=1
EnvironmentFile=-${envFile}
Restart=on-failure
RestartSec=2
KillMode=control-group

[Install]
WantedBy=default.target
`;
}
