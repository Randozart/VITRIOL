// The batch-eval topic set. Round 1 = the 10 `round:1` topics (survey +
// refinement mining); Round 2 = all 15 (the 10 re-run + 5 `round:2` additions),
// after refinements are applied. Deliberately diverse in shape — comparison,
// factual/current, broad survey, niche-technical, decision/how-to — with several
// in the user's local-AI domain.

export interface Topic {
  id: string;
  topic: string;
  round: 1 | 2;
}

export const TOPICS: Topic[] = [
  // ---- Round 1 (10) ----
  { id: "01-rust-vs-go-cli", round: 1, topic: "Compare the Rust and Go programming languages for building command-line (CLI) tools in 2026: performance, developer experience, ecosystem, binary distribution, and when to choose each." },
  { id: "02-local-coding-llms", round: 1, topic: "What are the most capable open-weight local LLMs for coding as of 2026, and what are their approximate VRAM requirements at common quantizations?" },
  { id: "03-rag-techniques", round: 1, topic: "Survey the current state of retrieval-augmented generation (RAG) techniques in 2026 and their tradeoffs (naive RAG, hybrid search, reranking, GraphRAG, agentic retrieval)." },
  { id: "04-vector-dbs", round: 1, topic: "Compare the leading self-hostable vector databases (pgvector, Qdrant, Milvus, Weaviate) for a small self-hosted setup: performance, features, operational complexity, and cost." },
  { id: "05-intermittent-fasting", round: 1, topic: "What is the current scientific consensus on the health effects of intermittent fasting, and where is the evidence still uncertain?" },
  { id: "06-transformer-evolution", round: 1, topic: "How did the transformer architecture evolve from the 2017 'Attention Is All You Need' paper to 2026 (key architectural changes and why they were adopted)?" },
  { id: "07-home-server-security", round: 1, topic: "What are the best practices for securing a self-hosted home server that is exposed to the internet in 2026?" },
  { id: "08-inference-servers", round: 1, topic: "Compare llama.cpp, vLLM, and TGI for serving local LLM inference: throughput, latency, hardware fit, feature set, and ideal use cases." },
  { id: "09-ai-regulation", round: 1, topic: "What is the state of AI regulation in the EU and the United States as of 2026, and what are the key obligations for developers of AI systems?" },
  { id: "10-ai-dev-laptop", round: 1, topic: "What should someone consider when buying a laptop for local AI/ML development in 2026 (GPU/VRAM, CPU, RAM, thermals, value)?" },

  // ---- Round 2 additions (5) ----
  { id: "11-zig-rust-c", round: 2, topic: "Compare Zig, Rust, and C for low-level systems programming in 2026: safety, performance, tooling, interop, and maturity." },
  { id: "12-microservices-vs-monolith", round: 2, topic: "What does the evidence say about microservices versus a monolith for a small engineering team, and how should they decide?" },
  { id: "13-reduce-hallucination", round: 2, topic: "What are the current, effective approaches to reducing hallucination in large language models (2026)?" },
  { id: "14-coding-agents", round: 2, topic: "How do modern coding agents (Claude Code, Cursor, Aider) differ in architecture and approach?" },
  { id: "15-finetune-small-models", round: 2, topic: "What are the best strategies for fine-tuning small local LLMs on a single consumer GPU (LoRA/QLoRA, data, memory tricks) in 2026?" },
];

export function topicsForRound(round: 1 | 2): Topic[] {
  // Round 1 runs the 10; round 2 runs all 15 (re-run + additions).
  return round === 1 ? TOPICS.filter((t) => t.round === 1) : TOPICS;
}
