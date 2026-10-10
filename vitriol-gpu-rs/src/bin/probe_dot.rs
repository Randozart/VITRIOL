// PROVENANCE: inspiration - NVlabs/cuda-oxide (Apache-2.0); API pattern from
// its examples. VITRIOL spike code: probe-shaped batched dot-product kernel
// (q[H,D] . k[N,H,D] -> scores[N,H]) with CPU parity + launch/bench timing.

use cuda_core::simt::LaunchConfig;
use cuda_core::{CudaContext, DeviceBuffer};
use cuda_device::{DisjointSlice, kernel, thread};
use cuda_host::{cuda_module, load_kernel_module};

#[cuda_module]
mod kernels {
    use super::*;

    /// One thread per (token, head): scores[n,h] = dot(q[h,:], k[n,h,:]).
    /// Same math shape as the lull KV probe scoring tail.
    #[kernel]
    pub fn probe_dot(
        q: &[f32],
        k: &[f32],
        mut out: DisjointSlice<f32>,
        n_head: u32,
        dim: u32,
    ) {
        let idx = thread::index_1d();
        let i = idx.get() as u32;
        if let Some(o) = out.get_mut(idx) {
            let _n = i / n_head;
            let h = i % n_head;
            let qo = (h * dim) as usize;
            let ko = (i * dim) as usize;
            let mut acc = 0.0f32;
            for d in 0..dim as usize {
                acc += q[qo + d] * k[ko + d];
            }
            *o = acc;
        }
    }
}

const N: u32 = 16_384; // tokens
const H: u32 = 8; // heads
const D: u32 = 64; // head dim

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = CudaContext::new(0)?;
    let mut data = ProbeData::new(&ctx)?;
    data.launch()?;
    check_parity(&data)?;
    run_bench(&mut data)?;
    Ok(())
}

/// Device + host buffers for one probe-shaped dataset.
struct ProbeData {
    typed: kernels::LoadedModule,
    stream: std::sync::Arc<cuda_core::CudaStream>,
    q_dev: DeviceBuffer<f32>,
    k_dev: DeviceBuffer<f32>,
    out_dev: DeviceBuffer<f32>,
    q: Vec<f32>,
    k: Vec<f32>,
}

impl ProbeData {
    fn new(ctx: &std::sync::Arc<CudaContext>) -> Result<ProbeData, Box<dyn std::error::Error>> {
        let stream = ctx.default_stream();
        let module = load_kernel_module(ctx, "probe_dot")
            .map_err(|e| format!("load kernel module: {e}"))?;
        let typed = kernels::from_module(module).map_err(|e| format!("typed module: {e}"))?;
        // deterministic inputs
        let q: Vec<f32> = (0..(H * D)).map(|i| ((i % 17) as f32) * 0.25 - 2.0).collect();
        let k: Vec<f32> = (0..(N * H * D)).map(|i| ((i % 23) as f32) * 0.125 - 1.5).collect();
        let q_dev = DeviceBuffer::from_host(&stream, &q)?;
        let k_dev = DeviceBuffer::from_host(&stream, &k)?;
        let out_dev = DeviceBuffer::<f32>::zeroed(&stream, (N * H) as usize)?;
        Ok(ProbeData { typed, stream, q_dev, k_dev, out_dev, q, k })
    }

    /// One kernel launch; SAFETY: 1-D, one thread per (n,h), buffers match.
    fn launch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        unsafe {
            self.typed.probe_dot(
                &self.stream,
                LaunchConfig::for_num_elems(N * H),
                &self.q_dev,
                &self.k_dev,
                &mut self.out_dev,
                H,
                D,
            )
        }?;
        Ok(())
    }

    fn synchronize(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(self.stream.synchronize()?)
    }
}

/// CPU parity for every (n, h) score against the last GPU result.
fn check_parity(data: &ProbeData) -> Result<(), Box<dyn std::error::Error>> {
    let out = data.out_dev.to_host_vec(&data.stream)?;
    let (q, k) = (&data.q, &data.k);
    let nu = N as usize;
    let hu = H as usize;
    let du = D as usize;
    let mut bad = 0usize;
    for idx in 0..nu * hu {
        let h = idx % hu;
        let qo = h * du;
        let ko = idx * du;
        let acc: f32 = (0..du).map(|d| q[qo + d] * k[ko + d]).sum();
        if (out[idx] - acc).abs() > 1e-3 {
            bad += 1;
        }
    }
    if bad != 0 {
        return Err(format!("parity: {bad}/{} mismatches", N * H).into());
    }
    println!("parity ok: probe_dot(N={N}, H={H}, D={D})");
    Ok(())
}

/// Timed launches, sync per batch.
fn run_bench(data: &mut ProbeData) -> Result<(), Box<dyn std::error::Error>> {
    for rep in [10u32, 100] {
        bench_rep(data, rep)?;
    }
    Ok(())
}

/// One bench batch: `rep` launches + one sync.
fn bench_rep(data: &mut ProbeData, rep: u32) -> Result<(), Box<dyn std::error::Error>> {
    let t0 = std::time::Instant::now();
    for _ in 0..rep {
        data.launch()?;
    }
    data.synchronize()?;
    let dt = t0.elapsed();
    let macs = (N as u64) * (H as u64) * (D as u64) * (rep as u64);
    println!(
        "bench {rep}x: {:?} ({:.2} us/launch, {:.1} GMAC/s)",
        dt,
        dt.as_micros() as f64 / rep as f64,
        macs as f64 / dt.as_secs_f64() / 1e9
    );
    Ok(())
}
