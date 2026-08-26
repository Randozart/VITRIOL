# VITRIOL Implementation Plan
## "Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"

**Machine**: Primary driver (HIGH CAUTION)  
**Philosophy**: Map territory first, push boundaries after verification  
**Timeframe**: however long it takes to be certain  

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                    Python Agent Clients                          │
│            (LangChain, CrewAI, Custom Agents)                  │
└─────────────────────────┬──────────────────────────────────────┘
                          │ Unix Socket (/var/run/vitriol.sock)
                          ▼
┌──────────────────────────────────────────────────────────────────┐
│                    vitriol-daemon (Userspace)                    │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────────┐│
│  │   Socket    │  │   llama.cpp │  │       Layer Manager         ││
│  │   Server    │  │  Inference  │  │     (LRU, Streaming)        ││
│  └─────────────┘  └─────────────┘  └─────────────────────────────┘│
└─────────────────────────┬──────────────────────────────────────┘
                          │ ioctl / Character Device
                          ▼
┌──────────────────────────────────────────────────────────────────┐
│                    vitriol.ko (Kernel Module)                    │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────────┐│
│  │   Safety    │  │     DRM     │  │         DMA Engine          ││
│  │   Layer     │  │  (nvidia)   │  │      (dmaengine API)        ││
│  └─────────────┘  └─────────────┘  └─────────────────────────────┘│
└─────────────────────────┬──────────────────────────────────────┘
                          │ PCIe
                          ▼
┌────────────────┐     ┌────────────────┐
│      SSD       │────▶│   GPU VRAM     │
│   (Storage)    │ DMA │  (8GB GTX)     │
└────────────────┘     └────────────────┘
```

---

## Components

### 1. vitriol.ko (Kernel Module)
- GPU detection via DRM
- VRAM allocation/deallocation
- DMA transfer initiation
- Safety enforcement
- Character device (`/dev/vitriol`) for daemon communication

### 2. vitriol-daemon (Userspace)
- Unix socket server (`/var/run/vitriol.sock`)
- Protocol handling (JSON over socket)
- llama.cpp integration
- Layer management (LRU eviction)
- Model inference execution

### 3. libvitriol (Python Package)
- Socket client library
- Type definitions
- Async support

---

## Safety Levels

| Level | Operations | Risk |
|-------|-------------|------|
| `safety_level=1` | Read-only GPU queries, DRM copy | **LOW** |
| `safety_level=2` | + DMA writes (opt-in) | **MEDIUM** |
| `safety_level=3` | + Raw PCI access | **HIGH** |

**Default**: `safety_level=1` (read-only, safe)

---

## Socket API

### Connection
```bash
SOCKET_PATH="/var/run/vitriol.sock"
```

### Protocol
- **Type**: Stream socket (SOCK_STREAM)
- **Format**: Length-prefixed JSON messages
- **Request**: `{ "cmd": "CMD_NAME", "params": {...}, "id": 1 }`
- **Response**: `{ "status": "ok|error", "data": {...}, "id": 1 }`

### Commands

```json
// Query status
{ "cmd": "STATUS", "id": 1 }

// Load model to GPU
{ "cmd": "LOAD_MODEL", "params": { "path": "/path/to/model.gguf" }, "id": 2 }

// Run inference
{ "cmd": "INFER", "params": { "prompt": "Hello", "max_tokens": 100 }, "id": 3 }

// Stream layer (for larger models)
{ "cmd": "STREAM_LAYER", "params": { "layer_id": 0, "ssd_offset": 1024 }, "id": 4 }

// Evict layer (LRU)
{ "cmd": "EVICT_LAYER", "params": { "layer_id": 5 }, "id": 5 }

// Set safety level
{ "cmd": "SET_SAFETY", "params": { "level": 2 }, "id": 6 }
```

---

## Error Codes

| Code | Meaning | Safety Level |
|------|---------|--------------|
| `OK` | Success | - |
| `GPU_NOT_DETECTED` | PCI scan failed | 1+ |
| `BAR_MAP_FAILED` | Cannot map GPU BAR | 1+ |
| `MEMORY_FULL` | VRAM allocation failed | 1+ |
| `DMA_FAILED` | Transfer failed | 2+ |
| `WRITE_BLOCKED` | Write attempted at safety_level=1 | 1+ |
| `MODEL_LOAD_FAILED` | llama.cpp failed to load model | 1+ |
| `INFERENCE_FAILED` | llama.cpp inference error | 1+ |

---

## Implementation Phases

### Phase 1: Kernel Module + Daemon (Foundation)
**Goal**: Establish working socket API with llama.cpp inference

#### Components
1. **Kernel module**:
   - Character device (`/dev/vitriol`)
   - Safety level enforcement
   - GPU detection (DRM-based)
   - VRAM queries

2. **Daemon**:
   - Socket server
   - llama.cpp integration
   - Qwen model loading
   - Basic inference

3. **Python client**:
   - Socket client library
   - Type definitions

#### Files
```
vitriol_new_ffi.bv       # Brief source → kernel module
vitriol-daemon/          # Userspace daemon
├── main.rs             # Socket server + llama.cpp
└── Cargo.toml
libvitriol/             # Python package
├── __init__.py
├── client.py
└── types.py
```

#### Success Criteria
- [ ] Module loads without error
- [ ] Daemon connects to socket
- [ ] STATUS command returns GPU info
- [ ] Model loads via llama.cpp
- [ ] Inference produces correct output
- [ ] Clean shutdown/unload

---

### Phase 2: DMA Integration
**Goal**: Add DMA transfers for layer streaming

#### Components
1. **Kernel module**:
   - Linux dmaengine integration
   - Zero-copy buffer management
   - DMA transfer API

2. **Daemon**:
   - Layer streaming via DMA
   - LRU eviction

#### Success Criteria
- [ ] DMA engine initializes
- [ ] Layer transfer via DMA
- [ ] Throughput > 3 GB/s

---

### Phase 3: Performance Optimization
**Goal**: Raw PCI access for maximum throughput

#### Components
1. **Kernel module**:
   - Raw BAR access
   - Custom DMA ring
   - Write gates

#### Success Criteria
- [ ] Throughput > 5 GB/s
- [ ] All safety tests pass

---

## Directory Structure

```
linux-pipe-module/
├── vitriol_new_ffi.bv       # Brief source
├── vitriol_new_ffi.c         # Generated C
├── vitriol_new_ffi.ko        # Kernel module
├── vitriol-daemon/          # Rust daemon
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── libvitriol/             # Python package
│   ├── __init__.py
│   ├── client.py
│   └── types.py
├── test_vitriol.sh
├── IMPLEMENTATION_PLAN.md
└── README.md
```

---

## Dependencies

### Kernel Module
- Linux kernel headers (6.x)
- DRM subsystem
- dmaengine API

### Daemon (Rust)
- tokio (async runtime)
- serde (JSON)
- llama.cpp bindings via cpython

### Python Client
- Python 3.8+
- Standard library only (socket)

---

## Critical Notes

1. **Primary machine caution**: Always test with `safety_level=1` first
2. **Incremental testing**: Each phase requires all tests before advancing
3. **Logging**: Check `dmesg` after each operation
4. **Clean shutdown**: Always unload module cleanly
