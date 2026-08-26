# VITRIOL Project Structure

```
linux-pipe-module/
├── README.md                    # Project overview and quick start
├── LICENSE                      # Apache 2.0 License
├── .gitignore                   # Git ignore rules
│
├── # Phase 1: Operational (KoboldCPP Integration)
├── launch_vitriol.sh           # Unified launch script for KoboldCPP + VITRIOL shim
├── test_shim.py                # Integration test suite
├── safe_test_vitriol.sh        # Graduated testing script
│
├── libvitriol/                 # Python client library
│   ├── __init__.py             # Package initialization
│   ├── client.py               # Socket client for VITRIOL daemon
│   ├── types.py                # Type definitions
│   └── vitriol_shim.py         # **Phase 1**: KoboldCPP context rectifier
│
├── # Phase 2+: Kernel Module (Advanced)
├── vitriol_new_ffi.bv          # Brief source for kernel module
├── vitriol.c                   # Generated C code (from Brief)
├── vitriol.bv                  # Original Brief source
├── kernel.toml                 # FFI bindings for kernel module
├── Makefile                    # Kernel module build rules
│
├── vitriol-daemon/             # Rust userspace daemon (Phase 2+)
│   ├── src/
│   │   └── main.rs             # Socket server + layer manager
│   └── Cargo.toml
│
├── # Documentation
├── docs/
│   ├── VITRIOL_IMPLEMENTATION_PLAN.md   # Complete 4-phase plan
│   ├── KOBOLDCPP_INTEGRATION.md         # KoboldCPP integration guide
│   ├── PHASE1_COMPLETE.md               # Phase 1 operational status
│   ├── PHASE1_STATUS.md                 # Phase 1 development status
│   ├── VITRIOL_ARCHITECTURE.md          # Architecture overview
│   ├── IMPLEMENTATION_PLAN.md           # Original implementation plan
│   ├── BRIEF_KERNEL_SOLUTION.md         # Brief compiler approach
│   ├── BRIEF_COMPILER_CHECKLIST.md      # Brief compiler checklist
│   └── CHANGES.md                       # Changelog
│
├── # Build Artifacts (gitignored)
├── build/
│   ├── *.o                              # Object files
│   ├── *.ko                             # Kernel modules
│   ├── *.cmd                            # Build commands
│   └── *.symvers                        # Kernel symbols
│
└── # Examples & Test Cases (gitignored)
    └── artifacts/
        ├── test_simple.bv               # Simple test case
        ├── test_frgn.bv                 # FFI test
        ├── test_simple.c                # Generated C
        ├── moore_stream.c               # DMA streamer prototype
        └── pci_bindings.toml            # PCI binding config
```

## Key Directories

### `/libvitriol/` - Python Library
Contains the Phase 1 context rectifier shim and Phase 2+ client library.

### `/vitriol-daemon/` - Rust Daemon
Userspace daemon for socket server and layer management (Phase 2+).

### `/docs/` - Documentation
All markdown documentation files organized here.

### `/build/` - Build Artifacts
Compiled kernel modules, object files, and build metadata. Automatically generated, gitignored.

### `/artifacts/` - Examples & Tests
Test cases, examples, and prototype code. Gitignored.

## Quick Navigation

- **Getting Started**: See `README.md`
- **Phase 1 Setup**: Run `./launch_vitriol.sh`
- **Testing**: Run `python3 test_shim.py`
- **Kernel Development**: See `docs/VITRIOL_IMPLEMENTATION_PLAN.md`
