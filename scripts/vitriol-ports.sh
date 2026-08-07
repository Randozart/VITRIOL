# vitriol-ports.sh — single source of truth for VITRIOL stack ports (Tria Prima).
#
# Three live processes map onto the three alchemical principles; each port encodes
# an atomic transmutation <from>><to> (element atomic numbers):
#   gen       Sulfur  82->79  Pb -> Au   the Opus (lead -> gold)
#   hermetis  Mercury 79->80  Au -> Hg   gold returns to the mercurial flux
#   embed     Salt    47->79  Ag -> Au   a second Opus (silver -> gold)
#
# Renumbering history (rationale, do not delete):
#  - 2026-08-07: hermetis 8090 -> 7980, embed 8081 -> 4779, per the Tria Prima
#    scheme in .opencode/plans/2026-08-07-vitriol-guide-install.md. gen stays 8279
#    (the original lead->gold joke). Env overrides (VITRIOL_GEN_PORT etc) preserved.
# To revert: set the *_PORT defaults back to 8090 / 8081 and touch the copy sites.

VITRIOL_GEN_PORT="${VITRIOL_GEN_PORT:-8279}"
VITRIOL_HERM_PORT="${VITRIOL_HERM_PORT:-7980}"
VITRIOL_EMBED_PORT="${VITRIOL_EMBED_PORT:-4779}"