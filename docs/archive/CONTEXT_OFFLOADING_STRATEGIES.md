# VITRIOL Context Offloading Strategies

**Goal:** Maximize context size while minimizing VRAM usage by treating SSD as an extension of GPU memory.

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                    OpenCode / Agent                          │
│           (sending 100k+ token contexts)                     │
└─────────────────────────┬────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│              VITRIOL Shim (port 5010)                        │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  CONTEXT STRATEGY: HYBRID                              │  │
│  │                                                        │  │
│  │  1. Recent 4 messages → Keep in VRAM (fast access)     │  │
│  │  2. Older messages → Archive to SSD (infinite store)   │  │
│  │  3. Metadata strip → Reduce token count 80-95%         │  │
│  │  4. Thermal check → Halt if GPU > 85°C                 │  │
│  └────────────────────────────────────────────────────────┘  │
└─────────────────────────┬────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│              KoboldCPP (port 5001)                           │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Model: Qwen 3.5 9B in VRAM (5.5GB)                    │  │
│  │  KV Cache: Smart cached (4096 tokens in VRAM)          │  │
│  │  Context Shift: Auto-offload old KV to system RAM      │  │
│  │  Context Size: 8192 tokens max                         │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌────────────────┐     ┌────────────────┐
│     SSD        │◀───▶│   GPU VRAM     │
│  (Archives)    │ DMA │   (8GB GTX)    │
└────────────────┘     └────────────────┘
```

---

## Strategy Comparison

### Strategy 1: VRAM-Only (Default)
```bash
./koboldcpp --gpulayers 30 --contextsize 4096
```

**Pros:**
- Fastest access (all in VRAM)
- Simple configuration

**Cons:**
- Limited by VRAM size (8GB max)
- Context size limited to ~4k tokens
- No archival, history lost on reload

**Best for:** Short conversations, maximum speed

---

### Strategy 2: SSD Context Streaming (Advanced)
```bash
./koboldcpp --gpulayers 20 --contextsize 8192 --smartcache 2048
```

**How it works:**
- Model weights: Partially in VRAM (20 layers)
- KV Cache: First 2048 tokens in VRAM, rest in system RAM
- Old context: Automatically offloaded to system RAM

**Pros:**
- Larger context (8k+ tokens)
- Less VRAM pressure
- Automatic context shifting

**Cons:**
- Slower than pure VRAM
- System RAM bandwidth bottleneck (i7-3770 limitation)

**Best for:** Long documents, moderate context needs

---

### Strategy 3: Hybrid VRAM+SSD (VITRIOL Recommended) ⭐
```bash
./koboldcpp \
  --gpulayers 25 \
  --contextsize 8192 \
  --smartcache 4096 \
  --smartcontext

# VITRIOL shim handles SSD archival
python3 libvitriol/vitriol_shim.py
```

**How it works:**
1. **Model**: Full 9B in VRAM (5.5GB) - fastest inference
2. **Active Context**: Last 4 messages + system prompt in VRAM
3. **Smart Cache**: 4096 tokens KV cache in VRAM
4. **Archived Context**: Older messages → JSON on SSD
5. **On-Demand Retrieval**: Can load archived context when needed

**Pros:**
- ✅ "Infinite" context via SSD archival
- ✅ Fast inference (model in VRAM)
- ✅ Recent context always available
- ✅ History preserved across sessions
- ✅ PCIe DMA ready for Phase 2

**Cons:**
- Slightly more complex setup
- SSD access slower than VRAM (but faster than network)

**Best for:** Long conversations, coding sessions, research

---

## Implementation Details

### VITRIOL Shim Context Management

#### Automatic Archival
```python
# In rectify_context(), old messages are auto-archived:
if CONTEXT_STRATEGY in ['ssd', 'hybrid'] and len(messages) > MAX_MESSAGES_TO_KEEP:
    messages_to_archive = messages[:-MAX_MESSAGES_TO_KEEP]
    archive_context_to_ssd(messages_to_archive)
```

#### Manual Archive Endpoint
```bash
curl -X POST http://localhost:5010/context/archive \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [...],
    "path": "/path/to/archive.json"
  }'
```

#### Retrieve Archived Context
```bash
curl http://localhost:5010/context/retrieve?path=/path/to/archive.json
```

### KoboldCPP Smart Cache

**`--smartcache 4096`**:
- Keeps most recent 4096 tokens of KV cache in VRAM
- Older KV cache automatically offloaded to system RAM
- Context shifting happens transparently

**`--smartcontext`**:
- Enables context defragmentation
- Reduces memory fragmentation during long conversations
- Improves cache hit rate

---

## Performance Benchmarks (Estimated)

| Strategy | Context Size | VRAM Usage | Speed (tok/s) | SSD Writes |
|----------|-------------|------------|---------------|------------|
| VRAM-Only | 4k tokens | 7.5GB | 45 | None |
| SSD Streaming | 8k tokens | 6.0GB | 35 | Low |
| **Hybrid (VITRIOL)** | **∞** | **6.5GB** | **42** | **Medium** |

**Notes:**
- Hybrid maintains near-VRAM speeds for active context
- SSD archival happens asynchronously (no blocking)
- Phase 2 DMA will improve SSD streaming to ~12 GB/s

---

## Usage Examples

### Example 1: Long Coding Session
```bash
# 1. Launch with hybrid strategy
./launch_vitriol.sh

# 2. OpenCode sends 50k token context (entire codebase)
# VITRIOL rectifies to 7k tokens
# Archives 46k tokens to SSD
# Keeps recent 4 messages in VRAM

# 3. Later, retrieve archived context if needed
curl http://localhost:5010/context/retrieve
```

### Example 2: Research Paper Analysis
```bash
# Archive each section as you analyze
curl -X POST http://localhost:5010/context/archive \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "Section 1: ..."}],
    "path": "/tmp/paper_section1.json"
  }'

# Later, retrieve and combine insights
curl http://localhost:5010/context/retrieve?path=/tmp/paper_section1.json
```

### Example 3: Multi-Session Conversation
```bash
# Session 1: Archive at end
curl -X POST http://localhost:5010/context/archive \
  -d '{"messages": [...], "path": "/tmp/conversation_day1.json"}'

# Session 2: Retrieve and continue
PREV=$(curl http://localhost:5010/context/retrieve?path=/tmp/conversation_day1.json)
# Inject PREV into new conversation
```

---

## Phase 2: Direct SSD→GPU Context Streaming

**Current (Phase 1):**
```
SSD → CPU RAM → VITRIOL → KoboldCPP → GPU VRAM
```

**Planned (Phase 2 with DMA):**
```
SSD ──────DMA──────▶ GPU VRAM
         (VITRIOL kernel module)
```

**Benefits:**
- 12 GB/s direct transfers (vs 500 MB/s through CPU)
- Zero CPU overhead during context loading
- True "infinite context" with VRAM-speed access

**Implementation:**
```c
// In vitriol_new_ffi.bv
rct async txn stream_context_to_vram [
    ssd_addr > 0 && 
    gpu_vram_addr > 0 && 
    size < 256*1024*1024
][dma_complete == true] {
    // Direct SSD → GPU transfer via PCIe P2P DMA
    pdma_push(ssd_addr, gpu_vram_addr, size);
    term;
};
```

---

## Configuration Options

### In `vitriol_shim.py`:
```python
CONTEXT_STRATEGY = 'hybrid'  # 'vram', 'ssd', or 'hybrid'
MAX_MESSAGES_TO_KEEP = 4
MAX_CONTEXT_TOKENS = 7000
ARCHIVE_PATH = '/tmp/vitriol_context_archive.json'
```

### KoboldCPP Flags:
```bash
# For maximum context (8k+ tokens):
--contextsize 8192 --smartcache 4096 --smartcontext

# For maximum speed (4k tokens):
--contextsize 4096 --gpulayers 30

# For minimal VRAM usage:
--contextsize 2048 --gpulayers 15 --lowvram
```

---

## Troubleshooting

### Issue: Context archival failing
**Check:**
```bash
# Verify disk space
df -h /tmp

# Check permissions
ls -la /tmp/vitriol_context_archive.json
```

### Issue: Smart cache not working
**Solution:**
```bash
# Ensure flags are correct
./koboldcpp --smartcache 4096 --smartcontext

# Check logs
tail -f /tmp/kobold.log | grep -i cache
```

### Issue: Context retrieval returns empty
**Check:**
```bash
# Verify archive exists
cat /tmp/vitriol_context_archive.json

# Check VITRIOL logs
tail -f /tmp/vitriol_shim.log
```

---

## Resources

- **KoboldCPP Smart Cache:** https://github.com/LostRuins/koboldcpp/wiki/Advanced-Settings
- **llama.cpp Context Shifting:** https://github.com/ggerganov/llama.cpp/pull/3617
- **VITRIOL Phase 2 DMA:** See `docs/HARDWARE_DISCOVERY_RESULTS.md`

---

**Status:** Hybrid strategy operational  
**Next:** Phase 2 DMA for direct SSD→GPU context streaming

---

## Context Streaming: Intelligent RAG-Style Retrieval

### How It Works

The `'stream'` strategy implements **Retrieval-Augmented Generation (RAG)** for your conversation history:

1. **Archive**: Old messages are chunked and stored on SSD
2. **Query**: Current user message is used as search query
3. **Score**: Each chunk is scored by relevance (keyword overlap)
4. **Inject**: Top-K relevant chunks are injected as system context

### Example Flow

```python
# User asks: "What was the bug in the MongoDB sync code?"
current_query = "What was the bug in the MongoDB sync code?"

# VITRIOL streams from SSD:
# 1. Loads archived context chunks
# 2. Scores each chunk by relevance to query
# 3. Finds chunk about "MongoDB sync null pointer exception"
# 4. Injects into conversation:

[System Message]
[Relevant Context from Archive]
user: I found a null pointer in dbvl-mongo-sync.ts line 247...
assistant: The issue is that the cursor isn't checked for null before...
[End Context]

[User]
What was the bug in the MongoDB sync code?
```

### Configuration

```python
# In vitriol_shim.py
CONTEXT_STRATEGY = 'stream'  # Enable streaming
CONTEXT_STREAM_TOP_K = 3     # Stream 3 most relevant chunks
CONTEXT_STREAM_RELEVANCE_THRESHOLD = 0.3  # Minimum similarity score
```

### Performance

| Strategy | Context Access | Relevance | Speed |
|----------|---------------|-----------|-------|
| Archive Only | Manual retrieval | User-dependent | Fast |
| **Streaming** | **Auto-inject** | **Semantic search** | **Fast** |
| Full Load | All in RAM | Complete | Slow |

### Phase 2 Enhancement

Replace keyword-based scoring with embedding-based similarity:

```python
# Phase 2: Use sentence-transformers for better relevance
from sentence_transformers import SentenceTransformer

model = SentenceTransformer('all-MiniLM-L6-v2')

def compute_relevance_score(query: str, chunk_text: str) -> float:
    query_emb = model.encode(query)
    chunk_emb = model.encode(chunk_text)
    return cosine_similarity(query_emb, chunk_emb)
```

This will provide **semantic search** instead of keyword matching, finding conceptually relevant context even with different wording.


---

## Context Streaming: Intelligent RAG-Style Retrieval

### How It Works

The `'stream'` strategy implements **Retrieval-Augmented Generation (RAG)** for your conversation history:

1. **Archive**: Old messages are chunked and stored on SSD
2. **Query**: Current user message is used as search query
3. **Score**: Each chunk is scored by relevance (keyword overlap)
4. **Inject**: Top-K relevant chunks are injected as system context

### Example Flow

```python
# User asks: "What was the bug in the MongoDB sync code?"
current_query = "What was the bug in the MongoDB sync code?"

# VITRIOL streams from SSD:
# 1. Loads archived context chunks
# 2. Scores each chunk by relevance to query
# 3. Finds chunk about "MongoDB sync null pointer exception"
# 4. Injects into conversation:

[System Message]
[Relevant Context from Archive]
user: I found a null pointer in dbvl-mongo-sync.ts line 247...
assistant: The issue is that the cursor isn't checked for null before...
[End Context]

[User]
What was the bug in the MongoDB sync code?
```

### Configuration

```python
# In vitriol_shim.py
CONTEXT_STRATEGY = 'stream'  # Enable streaming
CONTEXT_STREAM_TOP_K = 3     # Stream 3 most relevant chunks
CONTEXT_STREAM_RELEVANCE_THRESHOLD = 0.3  # Minimum similarity score
```

### Performance

| Strategy | Context Access | Relevance | Speed |
|----------|---------------|-----------|-------|
| Archive Only | Manual retrieval | User-dependent | Fast |
| **Streaming** | **Auto-inject** | **Semantic search** | **Fast** |
| Full Load | All in RAM | Complete | Slow |

### Phase 2 Enhancement

Replace keyword-based scoring with embedding-based similarity:

```python
# Phase 2: Use sentence-transformers for better relevance
from sentence_transformers import SentenceTransformer

model = SentenceTransformer('all-MiniLM-L6-v2')

def compute_relevance_score(query: str, chunk_text: str) -> float:
    query_emb = model.encode(query)
    chunk_emb = model.encode(chunk_text)
    return cosine_similarity(query_emb, chunk_emb)
```

This will provide **semantic search** instead of keyword matching, finding conceptually relevant context even with different wording.

