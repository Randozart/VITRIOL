# Token- and compute-efficient decisions for VITRIOL

Created: 2026-11-15T12:18:38+01:00 (Europe/Amsterdam).
Research status: candidate assessment and experiment proposal; no model adoption.
Local baseline: `784f39c77320d7eea548a507fcb11d20da9156c4`.
Primary sources below were inspected on 2026-11-15; mutable upstream branches must
be pinned before any dependency or weight installation.

## Question and constraints

Which decisions currently left to a large language model can be made with less
total work, while preserving decision quality, context and the main model's KV
residency? The user asked about Jev-style local alternatives, inexpensive input
processing, reusing existing cache, Mellum, and options beyond Laya.

The objective is total resource cost at an explicitly chosen error/fallback
budget. Counting output tokens alone does not capture input encoding, CPU/RAM
pressure, queueing, model loading, cache restoration or lost main-model reuse.
No architecture receives an accuracy or latency qualification from its name,
parameter count, output schema, advertised confidence or synthetic lab fixture.

Preserve these boundaries:

- Permissions, ownership/retirement predicates, retry protocol and known memory
  arithmetic remain deterministic code. A semantic classifier cannot authorize
  an otherwise invalid action.
- Input preparation must not conceal an additional large-LM summarization call.
  Use structured facts already maintained by code and relevant new text.
- Insufficient input coverage or uncertain predictions cause abstention/fallback;
  silently truncating important evidence is not an optimization.
- Candidate tests must preserve the active model/session. No model swap or KV
  disruption is justified by a small classifier's nominal throughput.

## Local observations

1. Officina's [auto-route classifier](../../officina/.pi/extensions/auto-route/classifier.ts)
   already computes heuristic complexity/privacy signals without model inference:
   prompt size, recent errors, churn, files touched, turn count and domain terms.
   Replacing that function with a model does not itself eliminate an LM call.
   Candidate decision points need an inventory of actual model calls and cost.
2. [Saved Mellum2 measurements](mellum2-highquant-consolidated-2026-08-19.md)
   demonstrate historical generation throughput and model residency constraints.
   They do not measure our proposed cold/warm classification latency or accuracy.
   Mellum2's active-expert count does not describe its full resident weight cost.
3. Hermes has [embedding storage operations](../../libhermes/src/db.rs), scoped by
   project and content hash. This is useful plumbing, not a validated shared
   classifier feature service. The [Rust context-block path](../../libhermes/src/retrieval.rs)
   explicitly reports embeddings unavailable for its topic heuristic. Any new
   encoder cache needs encoder/version/processing identity, not content alone.
4. The [optimization lab](../../docs/optimization-lab.md) and
   [acceptance results](2026-11-15-lab-acceptance-results.md) verify synthetic replay,
   governor policy replay and numerical runner behavior. They do not evaluate
   semantic classifiers, Laya accuracy, Mellum decision speed or GPU inference.
5. The recorded capability probe found no Laya/AnyJev packages or Laya weights.
   A separate CPU Python environment has PyTorch/Transformers; model/package
   download probes failed DNS resolution. No candidate benchmark was run.

## Mechanisms and candidates

| Mechanism | Work paid on each decision | Reuse opportunity | Main limitation |
|---|---|---|---|
| Rules; logistic/tree classifier on structured features | Feature lookup and a small prediction; no LM token processing | Existing state/counters | Fixed supported questions and, for learned heads, labelled outcomes |
| fastText-style supervised text classifier | Word/subword feature processing and category scoring; no transformer prefill | Stable category model; optional repeated-result cache | Lexical/contextual capability must suffice for the task |
| Shared small sentence encoder plus task-specific heads | Encode changed relevant text; cheap predictions from its vector | Same frozen encoder output can serve several trained heads | Labels/training and evidence representation are task-specific |
| Laya or schema-driven GLiNER2 classification | Encoder computation over the required text/question/schema | Exact compatible inputs/features; batch supported questions | Avoiding generated prose does not eliminate input computation |
| Mellum2 or another causal decoder used as a chooser | Uncached prefix/suffix processing, final-position label scoring | Its own compatible KV prefix | Residency, queueing, label accuracy and preserving session state |
| Readout attached to an already-computed actor representation | A small trained head if suitable features are captured during normal work | Existing hidden representations | Research adaptation; KV alone is not a ready general-purpose decision embedding |

The final row is a possible research direction, not an implemented interface or
an assertion that the model's hidden state contains sufficient decision evidence.
It cannot avoid work that has not happened yet, such as routing before a new
request has been encoded.

### Jev, Laya, AnyJev and local serving

[TypeSafe Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
offers typed choices/scores/judgments. Its publisher's speed and calibration claims
are not VITRIOL measurements. Structured output constrains representation;
semantic decisions can still be wrong.

[Laya's model card](https://huggingface.co/convaiinnovations/laya) identifies the
English ModernBERT-large checkpoint as 421M parameters and describes typed outputs.
Its [backend implementation](https://github.com/NandhaKishorM/laya/blob/main/laya/backends/__init__.py)
offers an explicit CPU ONNX path. Its [model/sequence implementation](https://github.com/NandhaKishorM/laya/blob/main/laya/common.py)
combines question/options/state for a bidirectional encoder and a decision head.
Consequently ordinary causal append-only KV reuse is not a drop-in optimization:
changed inputs can change earlier contextual representations. Question-token
reuse or runtime compilation caches are distinct from reusing neural KV/state.

[AnyJev](https://github.com/nokia-applied-research/AnyJev) provides decision readout
from open language models. Its [Transformers shared-prefix implementation](https://github.com/nokia-applied-research/AnyJev/blob/main/anyjev/backends/hf.py)
computes a prefix and branches suffix scoring from its KV, with token-boundary
checks. This inspected mechanism shares work within grouped scoring; it is not
evidence that all successive requests retain a persistent session cache.
The [inspected llama.cpp backend](https://github.com/nokia-applied-research/AnyJev/blob/main/anyjev/backends/llamacpp.py)
clears KV before each prompt and reads final-position label log probabilities.
Adopting that backend unchanged would lose the warm-cache benefit proposed here.

[Ollaya](https://github.com/ollaya-dev/ollaya) is a local serving option for open
decision models. API compatibility is not a compute/accuracy result. Model and
runtime licences/revisions need checking individually before adoption; no external
implementation or weights from these projects are incorporated in VITRIOL.

### Mellum and existing decoder cache

[Mellum2 Instruct](https://huggingface.co/JetBrains/Mellum2-12B-A2.5B-Instruct)
is a plausible bounded-choice baseline, particularly when already resident. It
could score allowed label tokens or return a short constrained label instead of
an explanation. Labels must be tokenized/compared correctly; raw label scores or
grammar-renormalized probabilities do not establish calibrated confidence.

[Causal KV caching](https://huggingface.co/docs/transformers/main/en/cache_explanation)
reuses computations for an unchanged prefix under compatible model state. Mellum
does not directly consume Qwen's KV; weights, adapters, positions, masks, rendered
token prefix and state/layout compatibility matter. A warm cache still costs
attention work for new tokens. Changing early instructions can destroy the shared
prefix even when the rest of the conversation looks identical to the operator.

The already-resident main model is therefore another relevant baseline. A short
decision suffix may reuse its context, but preserving/forking/restoring that
state must be verified first. The [engine cache integration](../../engine/tools/server/server-task.cpp)
retains identity gates; it does not establish a free, read-only decision fork.
A classifier query that triggers a later full prefill can erase the savings.

### Small shared encoder and SetFit-style heads

[SetFit](https://huggingface.co/docs/setfit/index) is a prompt-free classification
training framework. Its [classification-head documentation](https://huggingface.co/docs/setfit/en/how_to/classification_heads)
separates a sentence-embedding body from a classification head, using logistic
regression by default. The proposed VITRIOL adaptation is one shared frozen
encoder plus several heads trained on that encoder's representations. Independently
fine-tuning separate encoder bodies would defeat cross-head embedding reuse.

[MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) is a
concrete lightweight sentence-embedding baseline. The card specifies default
truncation beyond 256 wordpieces. It should receive explicitly bounded text, or
stable chunks with coverage tracked; default truncation must not silently lose
task requirements. Independent chunk embeddings are not equivalent to full-context
reasoning about relationships across those chunks.

Cache entries should bind content, encoder revision, tokenizer, pooling,
normalization/quantization and chunking. Re-encode only invalidated/new content.
Heads and their calibration also need versioning and invalidation when the encoder
changes. This is ordinary feature reuse, not cross-model KV interchange.

### fastText and GLiNER2

[fastText's supervised classification tutorial](https://fasttext.cc/docs/en/supervised-tutorial.html)
describes training category predictors from labelled documents. It is a useful
cheap baseline for stable, narrow labels; it still processes text and may not
capture the semantics required for arbitrary agent questions. No VITRIOL accuracy
or latency is asserted.

[GLiNER2](https://github.com/fastino-ai/GLiNER2) supports schema-driven extraction
and classification. The [classification facade](https://github.com/fastino-ai/GLiNER2/blob/main/gliner2/classification/engine.py)
accepts precomputed encoder output; its schema compilation cache and input feature
cache are different mechanisms. The [scorer](https://github.com/fastino-ai/GLiNER2/blob/main/gliner2/classification/scoring.py)
uses encoded schema/label information, so arbitrary changed questions must not be
assumed compatible with previously computed features. CPU/runtime compatibility,
applicable checkpoint licence and real decision quality remain untested here.

## Proposed experiment order and tradeoffs

1. Inventory actual LM decision calls and collect compact input facts, outcomes,
   costs and failure consequences. Keep sensitive examples private. Do not count
   existing rule decisions as avoided LM calls.
2. For recurring fixed labels, compare the existing rules, a learned structured
   feature classifier, fastText, and a small shared encoder with trained heads.
   Use a common held-out set; establish task-specific error costs beforehand.
3. For changing natural-language criteria, compare Laya/GLiNER2 and a warm-cache
   causal chooser, including Mellum only when residency is feasible without
   sacrificing main-model reuse. Distinguish cold load, cold input and warm prefix.
4. Shadow candidates before they affect routing or review. Measure accuracy by
   decision/task slice, calibration, abstention, fallback frequency, token coverage,
   p50/p95 latency, CPU/RAM/VRAM and actual avoided LM work. Include option-order,
   quantization, label changes and distribution drift. Test cache hit/miss parity.
5. Accept only candidates that reduce total cost at the chosen error budget.
   Confidence thresholds need held-out qualification and requalification under
   drift. Keep deterministic restrictions outside model control.

The current recommendation narrows the earlier CPU-first Laya suggestion:
Laya remains a flexible typed-decision candidate, while fixed recurring VITRIOL
decisions should first test structured features and small shared encoders. We do
not yet have empirical evidence to declare any candidate the overall winner.

A useful accounting model is:

`expected cost = new-input encoding + decision head + fallback probability × LM cost + cache/residency disruption`

It must include input preparation. Estimate baseline and candidate costs on the
same tasks; avoid double-counting an embedding already computed for retrieval.
Potential wins are falsified by inadequate evidence coverage, unacceptable task
errors, frequent fallback, wrong cache invalidation, or extra load/restore work
that consumes the expected savings. A model that eliminates prose generation but
repeatedly encodes the same large state does not satisfy the objective by itself.

## Evidence boundaries and related records

This dossier combines inspected primary implementation/documentation, dated local
measurements and explicitly marked proposals. No external benchmark was reproduced
for these decision models. No weights/runtime were downloaded, classifier trained,
live GPU queried, or routing policy changed for this research record.

Coverage includes typed-decision interfaces, encoder and causal architectures,
actual cache handling, prompt-free supervised heads, lexical classifiers and local
feature/cache plumbing. This is a bounded mechanism-based search, not an exhaustive
model survey. GLiNER2 and fastText were screened at source/docs level; deployment
licence, pinned dependency and target-machine checks remain adoption obligations.

Related: [GPU-emulation research](2026-11-15-gpu-emulation-research.md),
[lab implementation plan](2026-11-15-102907-optimization-lab.md),
[acceptance boundaries](2026-11-15-lab-acceptance-results.md), and
[documentation work plan](2026-11-15-121555-decision-efficiency-research-record.md).
