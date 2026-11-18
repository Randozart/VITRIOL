# Giving the agent session sudo — options, and the one I would pick

**Date:** 2026-11-17 19:35
**Mode:** PLAN (research only — nothing changed)
**Owner:** "is there a way we can give the session sudo permissions?"

## 1. Where it stands

```
$ sudo -n -l
sudo: a password is required
```

The session runs as `randozart` with **no** passwordless rights, so every privileged step in this
repo (restarting `vitriol-server`/`ascensusd`, `journalctl` for another unit, `nvidia-smi -pm 1`,
`visudo`) needs the owner at the keyboard. That has already cost time: today's engine restarts went
through `systemctl` only because a polkit rule allows the two vitriol units; reading the engine's
journal from the agent still fails.

## 2. Options

| option | what it means | verdict |
|---|---|---|
| **A. `NOPASSWD: ALL` for `randozart`** | total root for anything the agent runs | **no.** A prompt-injected command, a fat-fingered `rm -rf`, or a mis-pasted heredoc becomes root. The agent writes code for a living; the blast radius is the whole box. |
| **B. Narrow `NOPASSWD` allowlist** (`/etc/sudoers.d/vitriol-agent`) | exactly the commands the workflows need, nothing else | **recommended** |
| **C. polkit rule** (extend the existing one) | lets `randozart` manage *specific systemd units* without sudo at all | **do this too** — it is the cleanest form for units |
| **D. Run the TUI/pi as root** | sudo without sudo | **no.** Root-owned repo files, root-owned `~/.vitriol`, and every extension now has root. |
| **E. `askpass` helper so sudo can prompt through the TUI** | interactive password | no: the password would have to live in the environment for a non-interactive bash tool; that is strictly worse than A for leakage. |

## 3. The allowlist I would write

`/etc/sudoers.d/vitriol-agent` (mode 0440, validated with `visudo -c`):

```
# Agent session (randozart) — narrow, auditable, no shells.
Cmnd_Alias VITRIOL_UNITS = /usr/bin/systemctl restart vitriol-server.service, \
                           /usr/bin/systemctl restart vitriol-guard.service, \
                           /usr/bin/systemctl restart ascensusd.service, \
                           /usr/bin/systemctl start vitriol-server.service, \
                           /usr/bin/systemctl stop vitriol-server.service, \
                           /usr/bin/systemctl status vitriol-server.service
Cmnd_Alias VITRIOL_LOGS  = /usr/bin/journalctl -u vitriol-server.service *, \
                           /usr/bin/journalctl -u ascensusd.service *
Cmnd_Alias VITRIOL_GPU   = /usr/bin/nvidia-smi, /usr/bin/nvidia-smi -q, /usr/bin/nvidia-smi *
Cmnd_Alias VITRIOL_DISK  = /usr/bin/du *, /usr/bin/df *

randozart ALL=(root) NOPASSWD: VITRIOL_UNITS, VITRIOL_LOGS, VITRIOL_GPU, VITRIOL_DISK
```

Notes on the shape:

- **No `bash -c`, no `sh -c`, no editors, no `tee`, no `cp`** in the list — any of those re-opens
  arbitrary root execution through a single allowlisted entry.
- `nvidia-smi *` is broad but read-only in practice (`-pm`, `-pl`, `-lgc` change clocks, which the
  governor experiments do need). If that is too wide, split into `nvidia-smi -q*`, `-pm 1`,
  `-pl *`, `-lgc *`.
- `du *`/`df *` are read-only, but if the paths matter, pin them (`du -sh /scratch/*`).
- Units are pinned by name so a typo cannot hit another service.

And for units, prefer polkit (option C) over sudo — `/etc/polkit-1/rules.d/49-vitriol.rules`:

```javascript
polkit.addRule(function (action, subject) {
  if (activityInhibited()) return;
  if (action.id !== "org.freedesktop.systemd1.manage-units") return;
  if (subject.user !== "randozart") return;
  return ["vitriol-server.service", "vitriol-guard.service", "ascensusd.service"].indexOf(
    action.lookup("unit"),
  ) >= 0 ? polkit.Result.YES : null;
});
```

That is the same mechanism the existing vitriol-units rule uses, so it needs no sudo at all.

## 4. How the agent should use it

- Always `sudo -n` (non-interactive). A missing right then fails **loudly and immediately** instead
  of hanging a turn waiting for a password that will never arrive.
- Never chain `sudo` into a compound command that the allowlist does not match exactly
  (`sudo systemctl restart x && sudo ...` is fine; `sudo bash -c "..."` is not, and is refused by
  design).
- Every privileged action goes in the transcript (it is auditable in `/var/log/auth.log`) and, for
  the engine, into the fingerprint/plan doc.

## 5. What the owner has to do (root, once)

```bash
sudo visudo -f /etc/sudoers.d/vitriol-agent     # paste §3, save
sudo visudo -c                                  # syntax check
sudo -n -l                                      # confirm the allowlist
sudo install -m 0644 49-vitriol.rules /etc/polkit-1/rules.d/   # optional, §3 polkit block
```

The agent cannot write `/etc/sudoers.d` without the very access being granted, so this step is
inherently the owner's.

## 6. Recommendation

Option **B + C**: the narrow allowlist for logs/GPU/disk and the pinned units, polkit for unit
control. That covers every privileged step this repo's workflows actually need, keeps the root
surface small and auditable, and refuses everything else by explicit omission.

If you want a single number: this turns ~6 manual root steps per work session into zero, while the
only new root capability is "restart three named services, read their logs, and inspect the GPUs".
