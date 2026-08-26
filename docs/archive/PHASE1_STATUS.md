# VITRIOL Phase 1 Status Report

**Date:** 2026-05-09  
**Status:** Code Complete, Awaiting Hardware Resources

---

## ✅ Completed Work

### 1. VITRIOL Context Rectifier (`vitriol_shim.py`)
- ✅ Thermal polling via `nvidia-smi` (Guardrail 4)
- ✅ Context truncation (max 4 messages, 7k tokens)
- ✅ Metadata stripping (`<reasoning>`, tool results)
- ✅ OpenAI-compatible proxy endpoint
- ✅ Health check endpoint

### 2. Brief Kernel Module Updates
- ✅ Updated `vitriol_new_ffi.bv` to use BAR 1 (256MB VRAM window)
- ✅ Added hardware constants (GPU device ID, aperture size)
- ✅ Updated init transaction for correct BAR mapping

### 3. Documentation
- ✅ `VITRIOL_IMPLEMENTATION_PLAN.md` - Complete 4-phase plan
- ✅ `KOBOLDCPP_INTEGRATION.md` - Integration guide
- ✅ `launch_vitriol.sh` - Automated launch script
- ✅ Hardware analysis (BAR sizes, IOMMU status, thermal sensors)

### 4. Test Infrastructure
- ✅ `test_shim.py` - Integration test suite
- ✅ `safe_test_vitriol.sh` - Graduated testing script
- ✅ All scripts updated to use port 5010

---

## ⚠️ Current Blocker: Insufficient RAM

### System Configuration
- **RAM:** 15GB total, ~9GB available
- **Swap:** 8GB (newly created)
- **Model Size:** 5.48GB (Qwen 3.5 9B Q4_K_M)
- **KoboldCPP RAM Usage:** ~6GB (model + KV cache + overhead)

### Problem
KoboldCPP is being killed by OOM killer during model load:
```
Out of memory: Killed process (koboldcpp)
total-vm: 5889288kB, anon-rss: 5077816kB
```

The model needs ~5.6GB RAM, but with desktop environment (X11, Firefox, OpenCode) using ~3-4GB, the system runs out of memory during loading.

---

## 🔧 Solutions (Choose One)

### Option A: Close Applications (Recommended)
Before running KoboldCPP:
1. Close Firefox/Chrome browsers
2. Close OpenCode
3. Close other unnecessary applications
4. Then run: `./launch_vitriol.sh`

This should free up 2-3GB, enough for KoboldCPP to load.

### Option B: Add More Swap
Create additional swap space:
```bash
sudo fallocate -l 8G /swapfile2
sudo chmod 600 /swapfile2
sudo mkswap /swapfile2
sudo swapon /swapfile2
```

Total swap would be 16GB (8GB existing + 8GB new).

### Option C: Use CPU-Only Mode
Run KoboldCPP without CUDA (slower but less RAM):
```bash
./koboldcpp --model Qwen_Qwen3.5-9B-Q4_K_M.gguf --usecpu --contextsize 2048 --port 5001
```

### Option D: Use Smaller Model
Download a smaller model (e.g., Qwen 2.5 3B or Phi-3 mini):
```bash
huggingface-cli download Qwen/Qwen2.5-3B-Instruct-GGUF \
  --include "qwen2.5-3b-instruct-q4_k_m.gguf" \
  --local-dir ~/Downloads/koboldCPP
```

---

## 📋 Next Steps (Once KoboldCPP Runs)

### 1. Test VITRIOL Shim
```bash
cd ~/Desktop/Projects/linux-pipe-module
python3 test_shim.py
```

Expected output:
```
✓ KoboldCPP: responding
✓ VITRIOL: responding
✓ Rectification: 90%+ reduction
✓ Inference: working
```

### 2. Test Context Rectification
```bash
curl -X POST http://localhost:5010/rectify \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {"role": "system", "content": "You are helpful"},
      {"role": "user", "content": "A" * 50000},
      {"role": "assistant", "content": "B" * 50000},
      {"role": "user", "content": "Short question"}
    ]
  }' | python3 -m json.tool
```

Expected: Messages reduced from 3 to 2, tokens reduced 90%+

### 3. Configure OpenCode
Point OpenCode to VITRIOL instead of KoboldCPP directly:
- **Base URL:** `http://localhost:5010`
- **Endpoint:** `/v1/chat/completions`

### 4. Test with Bloated Context
Try a request that would normally crash (389k characters):
- VITRIOL should rectify it to <7k tokens
- KoboldCPP should respond without OOM

---

## 🎯 Success Criteria

Phase 1 is complete when:
- [ ] KoboldCPP stays running on port 5001
- [ ] VITRIOL shim stays running on port 5010
- [ ] `test_shim.py` passes all tests
- [ ] OpenCode can connect to port 5010
- [ ] No OOM crashes with large contexts

---

## 📝 Technical Notes

### Working KoboldCPP Command
```bash
./koboldcpp \
  --model Qwen_Qwen3.5-9B-Q4_K_M.gguf \
  --usecuda \
  --gpulayers 30 \
  --contextsize 8192 \
  --quantkv 1 \
  --lowvram \
  --noavx2 \
  --usemmap \
  --port 5001
```

### VITRIOL Shim Port
- Changed from 5005 to 5010 to avoid conflicts
- Update all test scripts and OpenCode config accordingly

### Key Files Modified
- `vitriol_shim.py` - Added thermal polling, updated rectification
- `vitriol_new_ffi.bv` - BAR 1 mapping (256MB VRAM)
- `test_shim.py` - Port updated to 5010
- `safe_test_vitriol.sh` - Port updated to 5010
- `launch_vitriol.sh` - New unified launch script

---

## 🚀 Quick Start (Once RAM Issue Resolved)

```bash
# 1. Close unnecessary applications
# Close browser, OpenCode, etc.

# 2. Launch VITRIOL stack
cd ~/Desktop/Projects/linux-pipe-module
./launch_vitriol.sh

# 3. Test in another terminal
python3 test_shim.py

# 4. Configure OpenCode
# Point to: http://localhost:5010/v1/chat/completions
```

---

**Status:** Ready to test once RAM availability issue is resolved.
**Recommendation:** Close browser and other apps, then run `./launch_vitriol.sh`
