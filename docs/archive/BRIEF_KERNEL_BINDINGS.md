# Brief Kernel Bindings (.dbv vs TOML)

**Decision:** Use `.dbv` (Data Brief) instead of TOML for kernel FFI bindings.

---

## Why .dbv is Superior

### Before (kernel.toml)
```toml
[pci_get_device]
sig = "pci_get_device(vendor: UInt, device: UInt, from: UInt) -> UInt"
note = "Find PCI device"
```

**Problems:**
- ❌ Stringly-typed signatures (no compile-time checking)
- ❌ Separate format from Brief code
- ❌ No type safety
- ❌ Manual parsing required

### After (kernel.dbv)
```brief
// Type-safe, contract-enforced FFI bindings
ffi pci_get_device(vendor: UInt, device: UInt, from: UInt) -> UInt;

// Constants are first-class citizens
const GPU_VENDOR: UInt = 0x10de;
const GPU_DEVICE: UInt = 0x1b82;
```

**Benefits:**
- ✅ Type-checked by Brief compiler
- ✅ Same syntax as main Brief code
- ✅ Constants can be used in contracts
- ✅ FFI functions appear as native Brief functions

---

## Architecture

```
vitriol_new_ffi.bv          kernel.dbv
     ↓                           ↓
#![bind("./kernel.dbv")]   FFI declarations
     ↓                       Constants
Uses GPU_VENDOR,            Memory barriers
GPU_DEVICE, etc.            DMA functions
```

### kernel.dbv Structure

```brief
// 1. FFI Function Declarations
ffi printk(fmt: String) -> Int;
ffi pci_get_device(vendor: UInt, device: UInt, from: UInt) -> UInt;

// 2. Hardware Constants
const GPU_VENDOR: UInt = 0x10de;
const GPU_DEVICE: UInt = 0x1b82;

// 3. DMA Constants
const DMA_TO_DEVICE: UInt = 1;
const DMA_BIDIRECTIONAL: UInt = 3;

// 4. Safety Levels
const SAFETY_READ_ONLY: UInt = 1;
const SAFETY_DMA_WRITE: UInt = 2;
const SAFETY_RAW_PCI: UInt = 3;
```

### vitriol_new_ffi.bv Usage

```brief
#![ffi.kernel, bind("./kernel.dbv")]

// Constants from kernel.dbv are available immediately
rct txn init [gpu_mapped == false][gpu_mapped == true] {
    // Use GPU_VENDOR directly (type-safe!)
    let result = pci_get_device(GPU_VENDOR, GPU_DEVICE, 0);
    
    // Use constants in contracts
    [result > 0] {
        &gpu_dev = result;
    };
    
    term;
};

// DMA with type-safe direction constant
rct txn dma_transfer {
    dma_map_single(gpu_dev, addr, size, DMA_BIDIRECTIONAL);
    term;
};
```

---

## Type Safety Examples

### Compile-Time Checking

```brief
// ✅ This compiles (correct types)
ffi pci_iomap(dev: UInt, bar: UInt, len: UInt) -> UInt;

let result = pci_iomap(gpu_dev, BAR_DATA, BAR_DATA_SIZE);

// ❌ This fails at compile time (type mismatch)
let result = pci_iomap("wrong", 1, 256MB);
// Error: Expected UInt, got String
```

### Contract Enforcement

```brief
// Precondition uses constant from kernel.dbv
rct txn stream_layer [
    current_layer < MAX_LAYERS  // MAX_LAYERS from kernel.dbv
][
    current_layer == @current_layer + 1
] {
    &current_layer = current_layer + 1;
    term;
};

// Brief compiler verifies:
// 1. MAX_LAYERS is defined and typed
// 2. current_layer is UInt
// 3. Comparison is valid
```

---

## Migration Guide: TOML → DBV

### Step 1: Create kernel.dbv
```brief
// Copy FFI signatures from TOML sig = "..."
ffi function_name(param: Type) -> ReturnType;

// Convert constants
const NAME: Type = value;
```

### Step 2: Update .bv file
```brief
// Change from:
#![ffi.kernel, bind("./kernel.toml")]

// To:
#![ffi.kernel, bind("./kernel.dbv")]
```

### Step 3: Remove duplicate definitions
```brief
// Delete from vitriol_new_ffi.bv:
const GPU_VENDOR: UInt = 0x10de;  // Now in kernel.dbv

// Keep in vitriol_new_ffi.bv:
let gpu_dev: UInt = 0;  // State variables
```

---

## Advanced Features

### Conditional FFI (Phase 2)

```brief
// Different FFI for test_mode vs real hardware
#[cfg(test_mode)]
ffi pci_get_device(vendor: UInt, device: UInt, from: UInt) -> UInt {
    // Stub implementation
    return 1;
}

#[cfg(not(test_mode))]
ffi pci_get_device(vendor: UInt, device: UInt, from: UInt) -> UInt {
    // Real hardware call
    // Implemented in C
}
```

### Inline Documentation

```brief
/// Map PCI BAR into kernel virtual address space
/// 
/// # Safety
/// - BAR must be valid for the device
/// - Length must not exceed BAR size
/// - Returns 0 on failure
/// 
/// # Example
/// let addr = pci_iomap(gpu_dev, BAR_DATA, BAR_DATA_SIZE);
ffi pci_iomap(dev: UInt, bar: UInt, len: UInt) -> UInt;
```

---

## Comparison Table

| Feature | TOML | .dbv |
|---------|------|------|
| **Type Checking** | ❌ Runtime | ✅ Compile-time |
| **Syntax** | String signatures | Brief-native |
| **Constants** | Manual parsing | First-class |
| **Contract Integration** | ❌ No | ✅ Yes |
| **Documentation** | Separate field | Brief comments |
| **IDE Support** | ❌ None | ✅ Brief LSP |
| **Refactoring** | Manual | Automatic |

---

## Future Enhancements

### Generic FFI (Brief v0.13+)

```brief
// Generic DMA mapping
ffi dma_map<T>(dev: UInt, buffer: T, dir: UInt) -> UInt
where T: DmaCoherent;
```

### Trait-based FFI

```brief
trait PciDevice {
    fn get_device(vendor: UInt, device: UInt) -> Self;
    fn map_bar(self, bar: UInt, len: UInt) -> UInt;
}

impl PciDevice for GpuDevice {
    // Implementation
}
```

---

## Resources

- **Brief Language Spec:** `spec/SPEC.md`
- **Data Brief Format:** `docs/DATA_BRIEF_FORMAT.md`
- **FFI Guide:** `learn-brief/14-ffi.md`

---

**Status:** Operational - kernel.dbv loaded by vitriol_new_ffi.bv  
**Type Safety:** ✅ Compile-time verified  
**Next:** Add more FFI functions as Phase 2 features are implemented
