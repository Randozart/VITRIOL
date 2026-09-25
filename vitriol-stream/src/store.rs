//! On-disk store reader. Byte-compatible with vitriol-store.cpp:
//!
//! - two 4096 B superblocks at 0 and 4096; valid = magic 'VITS', version 1,
//!   IEEE crc32 over bytes 0..4091 == crc32 field; prefer the first.
//! - super.expert_count is REUSED as the tensor count.
//! - index zone at 2*4096: dense 32 B descriptors + u64 crc trailer, the
//!   zone padded to 512.
//! - expert slice: ABSOLUTE offset = desc.store_offset
//!   + expert_idx * desc.slice_bytes (store_offset is absolute; verified
//!   against store_expert_offset in vitriol-store.cpp).
//! - all reads O_DIRECT: fd, buffer, size and offset must be 512-aligned
//!   (padded slice reads satisfy size; buffers come from bounce/aligned
//!   allocations).

use crate::crc32;
use std::ffi::{c_char, CStr};
use std::fs::File;
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

pub const MAGIC: u32 = 0x5649_5453; // 'VITS'
pub const VERSION: u32 = 1;
pub const BLK: usize = 4096;
pub const INDEX_BASE: u64 = 2 * BLK as u64;
pub const ALIGN: usize = 512;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Desc {
    pub layer: u32,
    pub kind: u32,
    pub n_experts: u32,
    pub slice_bytes: u32,
    pub data_bytes: u32,
    _pad0: u32,
    pub store_offset: u64,
}

/// 4096-aligned byte buffer (O_DIRECT requires buffer alignment too).
pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
    layout: std::alloc::Layout,
}

// The buffer is owned exclusively; SIMD-free CRC/reads do not alias it.
unsafe impl Send for AlignedBuf {}
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    pub fn zeroed(len: usize, align: usize) -> AlignedBuf {
        let layout = std::alloc::Layout::from_size_align(len.max(1), align)
            .expect("AlignedBuf layout");
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        AlignedBuf { ptr, len, layout }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr, self.layout) };
    }
}

#[derive(Debug)]
pub enum StoreError {
    Open(std::io::Error),
    NotAStore,
    SuperCorrupt,
    IndexCorrupt,
    Io(String),
}

impl From<String> for StoreError {
    fn from(e: String) -> Self {
        StoreError::Io(e)
    }
}

pub struct Store {
    file: File,
    n_tensors: u32,
    descs: Vec<Desc>,
}

impl Store {
    pub fn open(path: &str) -> Result<Store, StoreError> {
        let file = File::options()
            .read(true)
            .custom_flags(libc::O_DIRECT)
            .open(path)
            .map_err(StoreError::Open)?;

        let s = pick_super(&file).ok_or(StoreError::NotAStore)?;
        if s.crc() != crc32(&s.as_bytes()[0..BLK - 4]) {
            return Err(StoreError::SuperCorrupt);
        }
        let n_tensors = s.expert_count();

        let index_bytes = n_tensors as usize * std::mem::size_of::<Desc>();
        let mut descs = Vec::with_capacity(n_tensors as usize);
        if n_tensors > 0 {
            let zone_bytes = crate::align_up(index_bytes + 8, ALIGN);
            let mut zone = AlignedBuf::zeroed(zone_bytes, ALIGN);
            pread_exact(&file, zone.as_mut_slice(), INDEX_BASE)?;

            let idx_bytes = &zone.as_slice()[0..index_bytes];
            let mut stored_crc = [0u8; 8];
            stored_crc.copy_from_slice(&zone.as_slice()[index_bytes..index_bytes + 8]);
            if u64::from(crc32(idx_bytes)) != u64::from_le_bytes(stored_crc) {
                return Err(StoreError::IndexCorrupt);
            }
            for i in 0..n_tensors as usize {
                let off = i * std::mem::size_of::<Desc>();
                let b = &idx_bytes[off..off + std::mem::size_of::<Desc>()];
                descs.push(Desc {
                    layer: u32::from_le_bytes(b[0..4].try_into().unwrap()),
                    kind: u32::from_le_bytes(b[4..8].try_into().unwrap()),
                    n_experts: u32::from_le_bytes(b[8..12].try_into().unwrap()),
                    slice_bytes: u32::from_le_bytes(b[12..16].try_into().unwrap()),
                    data_bytes: u32::from_le_bytes(b[16..20].try_into().unwrap()),
                    _pad0: u32::from_le_bytes(b[20..24].try_into().unwrap()),
                    store_offset: u64::from_le_bytes(b[24..32].try_into().unwrap()),
                });
            }
        }

        Ok(Store {
            file,
            n_tensors,
            descs,
        })
    }

    pub fn n_tensors(&self) -> u32 {
        self.n_tensors
    }

    pub fn desc(&self, tid: u32) -> Option<&Desc> {
        self.descs.get(tid as usize)
    }

    /// Find the tensor id by (layer, kind), mirroring the C++ binder lookup.
    pub fn find_tensor(&self, layer: u32, kind: u32) -> Option<u32> {
        self.descs
            .iter()
            .position(|d| d.layer == layer && d.kind == kind)
            .map(|p| p as u32)
    }

    /// Read one expert slice into `dst` (len must equal data_bytes).
    /// `scratch` must be 512-aligned with capacity >= slice_bytes; the read
    /// is padded to slice_bytes, the first data_bytes are copied to dst.
    pub fn read_expert_into(
        &self,
        tid: u32,
        expert_idx: u32,
        dst: &mut [u8],
        scratch: &mut AlignedBuf,
    ) -> Result<(), String> {
        let d = self
            .desc(tid)
            .ok_or_else(|| format!("tid {tid} out of range"))?;
        if expert_idx >= d.n_experts
            || d.data_bytes == 0
            || d.slice_bytes < d.data_bytes
            || dst.len() != d.data_bytes as usize
        {
            return Err(format!(
                "geometry mismatch tid={tid} e={expert_idx} dst={}",
                dst.len()
            ));
        }
        let off = d.store_offset + u64::from(expert_idx) * u64::from(d.slice_bytes);
        if scratch.len() < d.slice_bytes as usize {
            return Err("scratch smaller than slice_bytes".to_string());
        }
        pread_exact(&self.file, &mut scratch.as_mut_slice()[0..d.slice_bytes as usize], off)?;
        dst.copy_from_slice(&scratch.as_slice()[0..d.data_bytes as usize]);
        Ok(())
    }

    /// Fast path used by the bounce ring: `buf` is 512-aligned with length
    /// >= slice_bytes; the padded slice is read directly into it. No copy.
    /// The H2D then transfers only the first data_bytes (== expert_size).
    pub fn read_expert_padded(
        &self,
        tid: u32,
        expert_idx: u32,
        buf: &mut [u8],
    ) -> Result<(), String> {
        let d = self
            .desc(tid)
            .ok_or_else(|| format!("tid {tid} out of range"))?;
        if expert_idx >= d.n_experts
            || d.data_bytes == 0
            || d.slice_bytes < d.data_bytes
            || buf.len() < d.slice_bytes as usize
        {
            return Err(format!(
                "padded read geometry mismatch tid={tid} e={expert_idx} buf={}",
                buf.len()
            ));
        }
        let off = d.store_offset + u64::from(expert_idx) * u64::from(d.slice_bytes);
        pread_exact(&self.file, &mut buf[0..d.slice_bytes as usize], off)
    }
}

#[repr(C, align(4096))]
struct Super {
    bytes: [u8; BLK],
}

impl Super {
    fn as_bytes(&self) -> &[u8; BLK] {
        &self.bytes
    }
    fn magic(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0..4].try_into().unwrap())
    }
    fn version(&self) -> u32 {
        u32::from_le_bytes(self.bytes[4..8].try_into().unwrap())
    }
    fn expert_count(&self) -> u32 {
        u32::from_le_bytes(self.bytes[8..12].try_into().unwrap())
    }
    fn crc(&self) -> u32 {
        u32::from_le_bytes(self.bytes[BLK - 4..BLK].try_into().unwrap())
    }
    fn valid(&self) -> bool {
        self.magic() == MAGIC && self.version() == VERSION
    }
}

fn pick_super(file: &File) -> Option<Super> {
    for at in [0u64, BLK as u64] {
        let mut sb = unsafe { std::mem::zeroed::<Super>() };
        if pread_exact(file, &mut sb.bytes, at).is_ok() && sb.valid() {
            return Some(sb);
        }
    }
    None
}

/// Full pread at `off` with O_DIRECT semantics (512-aligned buf/off/len).
pub(crate) fn pread_exact(file: &File, buf: &mut [u8], off: u64) -> Result<(), String> {
    let fd = file.as_raw_fd();
    let mut done = 0usize;
    while done < buf.len() {
        let n = unsafe {
            libc::pread(
                fd,
                buf[done..].as_mut_ptr().cast(),
                buf.len() - done,
                (off + done as u64) as libc::off_t,
            )
        };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("pread at {off}+{done}: {e}"));
        }
        if n == 0 {
            return Err(format!("pread EOF at {off}+{done}"));
        }
        done += n as usize;
    }
    Ok(())
}

/// Parse a C string path. Used by the FFI layer.
pub fn path_from_c_ptr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) };
    let s = s.to_str().ok()?;
    if s.is_empty() {
        return None;
    }
    Some(s.to_string())
}
