//! The ONLY module allowed to call CUDA. Every call returns CUDA_SUCCESS (0)
//! or an error code; no call is silently ignored.

#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::sync::OnceLock;

pub type CUdeviceptr = usize;
pub type CUstream = *mut c_void;
pub type CUevent = *mut c_void;

pub const CUDA_SUCCESS: i32 = 0;
pub const CU_STREAM_NON_BLOCKING: u32 = 1;
pub const CU_EVENT_DISABLE_TIMING: u32 = 0x2;

// Runtime API (linked transitively: ggml-cuda links libcudart).
pub const CUDA_HOST_ALLOC_PORTABLE: usize = 0x1;
pub const CUDA_MEMCPY_DEVICE_TO_HOST: i32 = 2;

extern "C" {
    // Driver API (libcuda; current context of the calling thread).
    // Memory/memcpy entry points MUST use the _v2 names: cuda.h #defines the
    // plain names to _v2 for C++, while the plain (v1, legacy context API)
    // symbols fail with CUDA_ERROR_INVALID_CONTEXT (201) on primary contexts.
    fn cuMemAlloc_v2(dptr: *mut CUdeviceptr, bytes: usize) -> i32;
    fn cuMemFree_v2(ptr: CUdeviceptr) -> i32;
    fn cuCtxGetDevice(dev: *mut i32) -> i32;
    fn cuCtxGetCurrent(pctx: *mut usize) -> i32;
    fn cuStreamCreate(stream: *mut CUstream, flags: u32) -> i32;
    fn cuStreamSynchronize(stream: CUstream) -> i32;
    fn cuStreamWaitEvent(stream: CUstream, event: CUevent, flags: u32) -> i32;
    fn cuEventCreate(event: *mut CUevent, flags: u32) -> i32;
    fn cuEventRecord(event: CUevent, stream: CUstream) -> i32;
    fn cuEventQuery(event: CUevent) -> i32;
    fn cuEventSynchronize(event: CUevent) -> i32;
    fn cuMemcpyHtoDAsync_v2(
        dst: CUdeviceptr,
        src: *const c_void,
        bytes: usize,
        stream: CUstream,
    ) -> i32;
    fn cuMemcpyDtoH_v2(dst: *mut c_void, src: usize, bytes: usize) -> i32;
    fn cuMemcpyHtoD_v2(dst: usize, src: *const c_void, bytes: usize) -> i32;

    // Runtime API.
    fn cudaHostAlloc(ptr: *mut *mut c_void, size: usize, flags: usize) -> i32;
    fn cudaFreeHost(ptr: *mut c_void) -> i32;
    fn cudaMemcpy(
        dst: *mut c_void,
        src: *const c_void,
        bytes: usize,
        kind: i32,
    ) -> i32;
}

fn check(res: i32, what: &str) -> Result<(), String> {
    if res == CUDA_SUCCESS {
        Ok(())
    } else {
        Err(format!("{what} failed: CUDA error {res}"))
    }
}

pub fn cu_ctx_get_current(pctx: &mut usize) -> i32 {
    unsafe { cuCtxGetCurrent(pctx) }
}

pub fn ctx_device() -> Result<i32, String> {
    let mut dev: i32 = 0;
    check(unsafe { cuCtxGetDevice(&mut dev) }, "cuCtxGetDevice")?;
    Ok(dev)
}

pub fn mem_alloc(bytes: usize) -> Result<CUdeviceptr, String> {
    let mut ptr: CUdeviceptr = 0;
    check(unsafe { cuMemAlloc_v2(&mut ptr, bytes) }, "cuMemAlloc_v2")?;
    Ok(ptr)
}

pub fn mem_free(ptr: CUdeviceptr) {
    let _ = unsafe { cuMemFree_v2(ptr) };
}

pub fn stream_create() -> Result<CUstream, String> {
    let mut s: CUstream = std::ptr::null_mut();
    check(
        unsafe { cuStreamCreate(&mut s, CU_STREAM_NON_BLOCKING) },
        "cuStreamCreate",
    )?;
    Ok(s)
}

pub fn stream_sync(stream: CUstream) -> Result<(), String> {
    check(unsafe { cuStreamSynchronize(stream) }, "cuStreamSynchronize")
}

pub fn stream_wait_event(stream: CUstream, event: CUevent) -> Result<(), String> {
    check(
        unsafe { cuStreamWaitEvent(stream, event, 0) },
        "cuStreamWaitEvent",
    )
}

pub fn event_create() -> Result<CUevent, String> {
    let mut e: CUevent = std::ptr::null_mut();
    check(
        unsafe { cuEventCreate(&mut e, CU_EVENT_DISABLE_TIMING) },
        "cuEventCreate",
    )?;
    Ok(e)
}

pub fn event_record(event: CUevent, stream: CUstream) -> Result<(), String> {
    check(unsafe { cuEventRecord(event, stream) }, "cuEventRecord")
}

/// CUDA_SUCCESS (0) means the recorded work has completed.
pub fn event_query_done(event: CUevent) -> bool {
    let r = unsafe { cuEventQuery(event) };
    r == CUDA_SUCCESS
}

pub fn event_sync(event: CUevent) -> Result<(), String> {
    check(unsafe { cuEventSynchronize(event) }, "cuEventSynchronize")
}

pub fn memcpy_h_to_d_async(
    dst: CUdeviceptr,
    src: *const c_void,
    bytes: usize,
    stream: CUstream,
) -> Result<(), String> {
    check(
        unsafe { cuMemcpyHtoDAsync_v2(dst, src, bytes, stream) },
        "cuMemcpyHtoDAsync",
    )
}

/// Blocking H2D on the current context (legacy NULL stream semantics).
pub fn memcpy_sync_h_to_d(dst: CUdeviceptr, src: *const c_void, bytes: usize) -> Result<(), String> {
    check(
        unsafe { cuMemcpyHtoD_v2(dst, src, bytes) },
        "cuMemcpyHtoD_v2",
    )
}

pub fn memcpy_d_to_h(dst: *mut c_void, src: CUdeviceptr, bytes: usize) -> Result<(), String> {
    check(unsafe { cuMemcpyDtoH_v2(dst, src, bytes) }, "cuMemcpyDtoH_v2")
}

pub unsafe fn host_alloc(bytes: usize) -> Result<*mut c_void, String> {
    let mut p: *mut c_void = std::ptr::null_mut();
    check(
        unsafe { cudaHostAlloc(&mut p, bytes, CUDA_HOST_ALLOC_PORTABLE) },
        "cudaHostAlloc",
    )?;
    Ok(p)
}

pub unsafe fn host_free(ptr: *mut c_void) {
    let _ = unsafe { cudaFreeHost(ptr) };
}

pub fn memcpy_sync_d_to_h(dst: *mut c_void, src: CUdeviceptr, bytes: usize) -> Result<(), String> {
    check(
        unsafe { cudaMemcpy(dst, src as *const c_void, bytes, CUDA_MEMCPY_DEVICE_TO_HOST) },
        "cudaMemcpy",
    )
}

/// Process-lifetime diagnostics flags, read once.
#[derive(Clone, Copy)]
pub struct Diag {
    pub hash: bool,
    pub vram_hash: bool,
    pub strict: bool,
    pub verbose: bool,
}

pub fn diag() -> Diag {
    static D: OnceLock<Diag> = OnceLock::new();
    *D.get_or_init(|| Diag {
        hash: std::env::var_os("VITRIOL_LRU_HASH").is_some(),
        vram_hash: std::env::var_os("VITRIOL_LRU_VRAM_HASH").is_some(),
        strict: std::env::var_os("VITRIOL_LRU_STRICT").is_some(),
        verbose: std::env::var_os("VITRIOL_VERBOSE").is_some(),
    })
}

/// 64-bit FNV-1a, matching the C++ diagnostic hash.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}
