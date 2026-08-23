# Reference 7 — misc, experimental & evaluation flags

The remainder: evaluation harnesses, training-adjacent knobs, logging, and
experimental features registered in the common layer but rarely used by a
serving stack.

PROVENANCE: arg.cpp registrations; REBIS relevance noted per flag.

## Evaluation harnesses

`--hellaswag` + `--hellaswag-tasks N`: run the HellaSwag benchmark from the
CLI. Not part of serving; used upstream for quality regression tracking.

## Imatrix / quantization-prep

`--in-file(s)`, `-bf/--binary-file`, `-o/--output(-file)`,
`-ofreq/--output-frequency`, `--save-frequency`, `--val-split`,
`--positive-file/--negative-file`, `--pca-batch`: imatrix collection and
control-vector training data plumbing. Relevant to D2: if we ever build
calibration matrices for our own quantizations of harvested models, this is
the machinery.

## Lookup decoding

`-lcd/--lookup-cache-dynamic PATH`: dynamic lookup cache for
lookup-style speculative decoding — an alternative drafting source that
updates itself from generation. Untested on this rig alongside MTP.

## Logging & verbosity

`--log-disable`, `--log-file FNAME`, `-v/--verbose/--log-verbose`,
`--tensor-filter REGEX`: log routing and debug dumps. Verbose mode was our
instrument for tracing prompt-cache restore decisions during E3r debugging
(SLT_DBG lines expose n_past decisions invisible at default level).

## Diffusion / experimental generation

`--diffusion-alg-temp`: dream-style diffusion LM temperature. Present in the
common layer; no diffusion model served here.

## REBIS stance

None of these are wired into daily operation. They're catalogued because a
flag reference that silently omits flags teaches the wrong lesson: every
registered argument is discoverable via `--help`, and anything undocumented
here can be explored with `llama-server --help | grep -A2 FLAG`.

PROVENANCE: arg.cpp registrations, current fork tree (commit b78f27738+H1).
