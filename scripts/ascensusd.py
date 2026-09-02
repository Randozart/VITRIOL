#!/usr/bin/env python3
"""ascensusd (:8283) — shared cloud-escalation core for every local agent.

Single implementation of: Hermetis dedup → euro-budget gate (single-writer
ledger) → Gemini call → usageMetadata actuals → store-back. opencode's
copula.ts and the hermes 'ascensus' skill are both thin clients, so budget
accounting has exactly one writer and dedup is consistent across agents.

Endpoints:
  GET  /health
  GET  /budget                 current ledger state
  POST /escalate               {query, reasoning?, files?, agent?, project_id?}

Wire compat: escalation records are stored as
  "[ascensus] model=<m> agent=<a>\\n<query>\\n→\\n<answer>"
so copula.ts dedup parsing (prefix "[ascensus]", answer after "\\n→\\n")
matches byte-for-byte.

Degradation ladder: Hermetis down ⇒ dedup skipped (call marked uncached,
budget still enforced); Gemini down ⇒ error surfaced; budget exhausted ⇒
deterministic refusal text telling the agent to answer locally.
"""
import json
import os
import re
import sys
import time
import urllib.request
import urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

PORT = int(os.environ.get("ASCENSUSD_PORT", "8283"))
HERMETIS_URL = os.environ.get("COPULA_HERMETIS_URL", "http://127.0.0.1:7980")
GEMINI_KEY_FILE = os.path.join(os.path.expanduser("~"), ".vitriol", "secrets")

EUR_DAILY = float(os.environ.get("ASCENSUS_EUR_DAILY", "1.0"))
EUR_MONTHLY = float(os.environ.get("ASCENSUS_EUR_MONTHLY", "30.0"))
MAX_CALLS_DAY = int(os.environ.get("ASCENSUS_MAX_CALLS_DAY", "0"))  # 0 = off
BUDGET_OFF = os.environ.get("ASCENSUS_BUDGET_OFF") == "1"
DEDUP_MIN_SCORE = float(os.environ.get("ASCENSUS_DEDUP_MIN_SCORE", "0.6"))
LEDGER_PATH = os.path.join(os.path.expanduser("~"), ".vitriol",
                           "ascensus_budget.json")

# EUR per Mtok [input, output]; generic fallback for unlisted models.
PRICES = {
    "gemini-2.5-flash-lite": (0.10, 0.40),
    "gemini-2.5-flash": (0.28, 2.30),
    "gemini-2.0-flash-lite": (0.09, 0.37),
}
try:
    if os.environ.get("ASCENSUS_PRICES_JSON"):
        PRICES.update({k: tuple(v) for k, v in
                       json.loads(os.environ["ASCENSUS_PRICES_JSON"]).items()})
except Exception:
    pass
FALLBACK_PRICE = (0.30, 2.50)

SECRET_COMPONENTS = re.compile(
    r"^(\.env|\.git|\.ssh|\.aws|secrets|credentials|\.vitriol)$", re.I)
SECRET_BASENAME = re.compile(
    r"(api[_-]?key)|(^keys?\.(json|txt|ya?ml|toml|ini|conf)$)"
    r"|(\.(pem|p12|p8|jks|keystore|key)$)|(secret|credential)", re.I)


def log(msg):
    print(f"[ascensusd {time.strftime('%H:%M:%S')}] {msg}", flush=True)


def price_for(model):
    for k, v in PRICES.items():
        if k in model:
            return v
    return FALLBACK_PRICE


def load_ledger():
    now = time.gmtime()
    day = time.strftime("%Y-%m-%d", now)
    month = day[:7]
    led = {"day": day, "spent_eur": 0.0, "month": month,
           "spent_month_eur": 0.0, "calls": []}
    try:
        with open(LEDGER_PATH) as f:
            led = json.load(f)
    except Exception:
        pass
    if led.get("day") != day:
        led["day"], led["spent_eur"] = day, 0.0
    if led.get("month") != month:
        led["month"], led["spent_month_eur"] = month, 0.0
    return led


def save_ledger(led):
    led["calls"] = led.get("calls", [])[-200:]
    tmp = LEDGER_PATH + ".tmp"
    with open(tmp, "w") as f:
        json.dump(led, f)
    os.replace(tmp, LEDGER_PATH)


def read_secrets():
    key = os.environ.get("GEMINI_API_KEY", "")
    model = os.environ.get("GEMINI_MODEL", "")
    try:
        with open(GEMINI_KEY_FILE) as f:
            for line in f:
                m = re.match(r"\s*(\w+)\s*=\s*(.+?)\s*$", line)
                if not m:
                    continue
                k, v = m.groups()
                if k == "api_key" and not key:
                    key = v.strip().strip('"')
                elif k == "model" and not model:
                    model = v.strip().strip('"')
    except Exception:
        pass
    return key, model or "gemini-2.5-flash"


def http_json(url, payload=None, timeout=120):
    data = None
    headers = {"Content-Type": "application/json"}
    if payload is not None:
        data = json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def gate_files(files):
    """Return (blocks, sent_kb, rejected_names). Mirrors copula's rules."""
    blocks, kb, rejected = [], 0.0, []
    root_ok = True
    for f in (files or [])[:4]:
        p = f.get("path", "") if isinstance(f, dict) else str(f)
        comps = [c for c in re.split(r"[\\/]+", p) if c]
        base = comps[-1].lower() if comps else ""
        if any(SECRET_COMPONENTS.match(c) for c in comps) or \
                SECRET_BASENAME.search(base):
            rejected.append(p)
            continue
        fp = os.path.join(os.path.expanduser("~"), p.lstrip("/")) \
            if not os.path.isabs(p) else p
        try:
            size = os.path.getsize(fp)
            if size > 64 * 1024:
                rejected.append(p + " (oversize)")
                continue
            with open(fp, encoding="utf-8", errors="replace") as fh:
                body = fh.read()
            kb += len(body.encode()) / 1024
            reason = (f.get("reason") or "-") if isinstance(f, dict) else "-"
            lines = f.get("lines") if isinstance(f, dict) else None
            if isinstance(lines, list) and len(lines) == 2:
                ls = body.splitlines()
                a, b = max(0, int(lines[0]) - 1), min(len(ls), int(lines[1]))
                body = "\n".join(ls[a:b])
            blocks.append(f"--- {p} ({reason}) ---\n{body}")
        except Exception as e:
            rejected.append(f"{p} ({e})")
    if kb > 512:
        blocks = ["[file payload dropped: total over 512 KB wire guardrail]"]
    return "\n\n".join(blocks), kb, rejected


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):  # noqa: A002
        pass

    def do_GET(self):
        path = urlparse(self.path).path
        if path == "/health":
            self._send(200, {"ok": True, "service": "ascensusd"})
        elif path == "/budget":
            led = load_ledger()
            self._send(200, {
                "spent_eur_today": round(led["spent_eur"], 5),
                "cap_daily": EUR_DAILY,
                "spent_eur_month": round(led["spent_month_eur"], 5),
                "cap_monthly": EUR_MONTHLY,
                "calls_recorded": len(led.get("calls", [])),
                "disabled": BUDGET_OFF,
            })
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if urlparse(self.path).path != "/escalate":
            return self._send(404, {"error": "not found"})
        try:
            length = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(length) or b"{}")
        except Exception as e:
            return self._send(400, {"error": f"bad json: {e}"})

        query = (req.get("query") or "").strip()
        if not query:
            return self._send(400, {"error": "query required"})
        agent = (req.get("agent") or "unknown").strip()[:32]
        project_id = (req.get("project_id") or "default").strip()[:128]

        # Auto-route telemetry (optional, from the officina auto-route extension).
        # Logged and persisted in the ledger for threshold tuning; never enforced.
        def _clamp01(v):
            try:
                f = float(v)
                return min(1.0, max(0.0, f))
            except (TypeError, ValueError):
                return None
        complexity_score = _clamp01(req.get("complexity_score"))
        privacy_score = _clamp01(req.get("privacy_score"))
        signals = req.get("signals") if isinstance(req.get("signals"), dict) else {}
        if complexity_score is not None or privacy_score or signals:
            log(f"route signals for {agent}: complexity={complexity_score} "
                f"privacy={privacy_score} signals={json.dumps(signals)[:200]}")

        key, model = read_secrets()
        if not key:
            return self._send(200, {"status": "unconfigured",
                                    "message": "no GEMINI_API_KEY — answer locally"})

        # ── L1 dedup against prior escalations (free) ──
        cached = None
        try:
            sr = http_json(f"{HERMETIS_URL}/hermetis/search",
                           {"project_id": project_id, "query": query,
                            "top_k": 3}, timeout=6)
            for r in sr.get("results", []):
                c = str(r.get("content", ""))
                # Rust port prefixes retrieval results with "[date] role: "
                # so match [ascensus] ANYWHERE, not just at position 0.
                idx = c.find("[ascensus]")
                if r.get("score", 0) >= DEDUP_MIN_SCORE and idx >= 0 \
                        and "\n→\n" in c[idx:]:
                    cached = c[idx:].split("\n→\n", 1)[1].strip()
                    break
        except Exception as e:
            log(f"dedup skipped (Hermetis unreachable): {e}")
        if cached:
            log(f"dedup hit for {agent}")
            return self._send(200, {"status": "cached", "answer": cached[:8000],
                                    "eur_spent": 0.0})

        # ── assemble prompt ──
        reasoning = req.get("reasoning") or ""
        file_block, sent_kb, rejected = gate_files(req.get("files"))
        parts = [f"Workspace: {project_id}", "",
                 f"User inquiry: {query}", ""]
        if reasoning:
            parts += ["Local reasoning attempt:", reasoning, ""]
        if rejected:
            parts.append(f"Rejected files (secret/oversize): {rejected}")
        if file_block:
            parts += ["Attached context:", file_block]
        parts += ["", "Give a precise, decisive verdict. If ambiguous, state "
                      "the key assumption and proceed. Answer in JSON: "
                      '{"answer": <direct judgement>, "reasoning": <2-3 '
                      'sentences>, "confidence": <0..1>}.']
        user_text = "\n".join(parts)
        wire_tokens = len(user_text) // 4

        # ── euro-budget gate on worst-case estimate ──
        if not BUDGET_OFF:
            led = load_ledger()
            p_in, p_out = price_for(model)
            est = (wire_tokens * p_in + 2048 * p_out) / 1e6
            if (led["spent_eur"] + est > EUR_DAILY or
                    led["spent_month_eur"] + est > EUR_MONTHLY or
                    (MAX_CALLS_DAY and
                     sum(1 for c in led["calls"]
                         if time.strftime("%Y-%m-%d",
                                          time.localtime(c["ts"])) ==
                         time.strftime("%Y-%m-%d")) >= MAX_CALLS_DAY)):
                log(f"budget refusal for {agent} (est €{est:.4f})")
                return self._send(200, {
                    "status": "budget_exhausted",
                    "message": (
                        f"[Ascensus — euro budget exhausted. Spent "
                        f"€{led['spent_eur']:.3f}/{EUR_DAILY} today, "
                        f"€{led['spent_month_eur']:.2f}/{EUR_MONTHLY} this "
                        f"month; next call ≈ €{est:.4f}. Answer locally — do "
                        f"not retry ascensus until tomorrow.]"),
                    "eur_spent": 0.0})

        # ── Gemini call ──
        payload = {
            "contents": [{"parts": [{"text": user_text}]}],
            "generationConfig": {
                "maxOutputTokens": 2048,
                "responseMimeType": "application/json",
                "responseSchema": {
                    "type": "OBJECT",
                    "properties": {
                        "answer": {"type": "STRING"},
                        "reasoning": {"type": "STRING"},
                        "confidence": {"type": "NUMBER"}},
                    "required": ["answer", "reasoning", "confidence"]}}}
        try:
            res = http_json(
                f"https://generativelanguage.googleapis.com/v1beta/models/"
                f"{model}:generateContent?key={key}", payload)
        except urllib.error.HTTPError as e:
            detail = e.read()[:300].decode("utf-8", "replace")
            return self._send(200, {"status": "error",
                                    "message": f"Gemini HTTP {e.code}: {detail}"})
        except Exception as e:
            return self._send(200, {"status": "error",
                                    "message": f"Gemini failed: {e}"})

        raw = "".join(p.get("text", "") for p in
                      (res.get("candidates", [{}])[0].get("content", {})
                       .get("parts", []))).strip()
        answer = raw
        try:
            parsed = json.loads(raw)
            conf = parsed.get("confidence")
            answer = (parsed.get("answer", "") +
                      ("\n\n" + parsed["reasoning"] if parsed.get("reasoning") else "") +
                      (f"\n\n(confidence {conf})" if conf is not None else ""))
        except Exception:
            pass
        if not answer:
            return self._send(200, {"status": "error",
                                    "message": "Gemini returned no text."})

        # ── actuals → single-writer ledger ──
        um = res.get("usageMetadata", {}) or {}
        in_tok = int(um.get("promptTokenCount", wire_tokens))
        out_tok = int(um.get("candidatesTokenCount", 2048))
        p_in, p_out = price_for(model)
        eur = (in_tok * p_in + out_tok * p_out) / 1e6
        led = load_ledger()
        rec = {"ts": time.time(), "model": model,
               "in_tok": in_tok, "out_tok": out_tok,
               "eur": eur, "agent": agent}
        if complexity_score is not None:
            rec["complexity_score"] = complexity_score
        if privacy_score is not None:
            rec["privacy_score"] = privacy_score
        if signals:
            rec["signals"] = signals
        led["calls"].append(rec)
        led["spent_eur"] += eur
        led["spent_month_eur"] += eur
        save_ledger(led)
        log(f"escalated for {agent}: in={in_tok} out={out_tok} €{eur:.4f}")

        # ── learning loop: store so future dedup hits are free ──
        record = (f"[ascensus] model={model} agent={agent}\n{query}\n→\n"
                  f"{answer[:8000]}")
        try:
            http_json(f"{HERMETIS_URL}/hermetis/store",
                      {"project_id": project_id, "session_id":
                       f"ascensus-{time.strftime('%Y%m%d')}",
                       "role": "tool", "content": record[:16000]}, timeout=8)
        except Exception as e:
            log(f"store-back failed (escalation uncached): {e}")

        self._send(200, {"status": "escalated", "answer": answer[:8000],
                         "eur_spent": eur, "model": model})


if __name__ == "__main__":
    log(f"listening on 127.0.0.1:{PORT}")
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
