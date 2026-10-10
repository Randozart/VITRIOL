// PROVENANCE: inspiration - NVlabs/cuda-oxide (Apache-2.0); API pattern from
// its host_closure example (SPDX NVIDIA). This file is VITRIOL spike code.

//! cuda-oxide hello-world: 1-D map kernel, parity + launch overhead.
//! Build/run: `cargo oxide run` (repo: NVlabs/cuda-oxide, nightly-2026-08-28).

use cuda_core::simt::LaunchConfig;
use cuda_core::{CudaContext, DeviceBuffer};
use cuda_device::{DisjointSlice, kernel, thread};
use cuda_host::{cuda_module, load_kernel_module};

#[cuda_module]
mod kernels {
    use super::*;

    #[kernel]
    pub fn scale(input: &[f32], factor: f32, mut out: DisjointSlice<f32>) {
        let idx = thread::index_1d();
        let i = idx.get();
        if let Some(out_elem) = out.get_mut(idx) {
            *out_elem = input[i] * factor;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = CudaContext::new(0)?;
    let stream = ctx.default_stream();
    let module = load_kernel_module(&ctx, "vitriol_gpu_rs")
        .map_err(|e| format!("load kernel module: {e}"))?;
    let typed = kernels::from_module(module)
        .map_err(|e| format!("typed module: {e}"))?;

    run_parity(&stream, &typed)?;
    run_launch_overhead_bench(&stream, &typed)?;
    Ok(())
}

/// 1 Mi-element scale + CPU parity check.
fn run_parity(
    stream: &cuda_core::CudaStream,
    typed: &kernels::LoadedModule,
) -> Result<(), Box<dyn std::error::Error>> {
    const N: usize = 1 << 20;
    let factor = 2.5f32;
    let host_in: Vec<f32> = (0..N).map(|i| i as f32).collect();
    let dev_in = DeviceBuffer::from_host(stream, &host_in)?;
    let mut dev_out = DeviceBuffer::<f32>::zeroed(stream, N)?;

    // SAFETY: 1-D launch, one thread per element, both buffers cover N f32s.
    unsafe {
        typed.scale(
            stream,
            LaunchConfig::for_num_elems(N as u32),
            &dev_in,
            factor,
            &mut dev_out,
        )
    }?;
    let out = dev_out.to_host_vec(stream)?;
    let bad = out.iter().zip(&host_in).filter(|(o, x)| (**o - *x * factor).abs() > 1e-5).count();
    if bad != 0 {
        return Err(format!("parity: {bad}/{N} mismatches").into());
    }
    println!("parity ok: scale({N}, {factor})");
    Ok(())
}

/// Rough launch overhead: 100 tiny launches + one sync.
fn run_launch_overhead_bench(
    stream: &cuda_core::CudaStream,
    typed: &kernels::LoadedModule,
) -> Result<(), Box<dyn std::error::Error>> {
    const TINY: usize = 256;
    let factor = 2.5f32;
    let tiny_in = DeviceBuffer::<f32>::from_host(stream, &vec![1.0f32; TINY])?;
    let mut tiny_out = DeviceBuffer::<f32>::zeroed(stream, TINY)?;
    let t0 = std::time::Instant::now();
    for _ in 0..100 {
        // SAFETY: same 1-D contract as run_parity, TINY elements.
        unsafe {
            typed.scale(
                stream,
                LaunchConfig::for_num_elems(TINY as u32),
                &tiny_in,
                factor,
                &mut tiny_out,
            )
        }?;
    }
    stream.synchronize()?;
    println!("100 tiny launches + sync: {:?}", t0.elapsed());
    Ok(())
}
