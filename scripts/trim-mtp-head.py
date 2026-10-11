#!/usr/bin/env python3
"""
Self-trim MTP draft head: extract top-N columns of output.weight by L2 norm,
emit a draft-only GGUF (blk.64 MTP layer + trimmed head + d2t mapping +
full token_embd for input embedding).

Provenance: mechanism from "How We Made Qwen 3.8 27B Generate 2.4x Faster on
llama.cpp - Losslessly" (Sreesanth R / Allen Joseph G, 2026-10-08, FastMTP /
d2t draft-vocab trimming). This script applies the same d2t idea to our OWN
MTP head (option B in the FastMTP d2t probe plan), eliminating the external
draft dependency.

Usage:
  trim-mtp-head.py <model.gguf> <output.gguf> [n_trim=32768]

The output draft GGUF is loaded via --spec-type draft-mtp --spec-draft-model.
"""
import sys
import os
import numpy as np

sys.path.insert(0, "/home/randozart/Desktop/Projects/VITRIOL/llama.cpp/gguf-py")
from gguf import GGUFReader, GGUFWriter, GGMLQuantizationType, GGUFValueType
from gguf.quants import quantize, dequantize


def copy_metadata(reader, writer):
    """Copy all GGUF metadata fields from reader to writer."""
    for field in reader.fields.values():
        # suppress virtual fields and fields written by GGUFWriter
        if field.name == "general.architecture" or field.name.startswith("GGUF."):
            continue
        val_type = field.types[0]
        sub_type = field.types[-1] if val_type == GGUFValueType.ARRAY else None
        val = field.contents()
        writer.add_key_value(field.name, val, val_type, sub_type=sub_type)


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    model_path = sys.argv[1]
    out_path = sys.argv[2]
    n_trim = int(sys.argv[3]) if len(sys.argv) > 3 else 32768
    # selection mode: l2 (top-L2-norm — FAILED, kept for reference) |
    #   ext-d2t (token set from an external d2t draft, our weights) |
    #   bpe (low BPE ids + specials — self-derived frequency proxy)
    mode = sys.argv[4] if len(sys.argv) > 4 else "l2"
    # ext-d2t consumes argv[5] for the external draft path; other modes take
    # the head quant type there instead
    ext_draft = sys.argv[5] if (len(sys.argv) > 5 and mode == "ext-d2t") else None
    # head quant: Q8_0 default; Q4_0 halves draft-head bandwidth (FastMTP uses ~3-bit)
    head_qname = "Q8_0"
    if len(sys.argv) > 5 and mode != "ext-d2t":
        head_qname = sys.argv[5]
    elif len(sys.argv) > 6:
        head_qname = sys.argv[6]

    print(f"reading {model_path}")
    reader = GGUFReader(model_path)

    tensors = {t.name: t for t in reader.tensors}
    required = ["token_embd.weight", "output_norm.weight", "output.weight"]
    for r in required:
        if r not in tensors:
            print(f"ERROR: missing required tensor {r}")
            sys.exit(1)

    blk64 = [t for t in reader.tensors if t.name.startswith("blk.64.")]
    print(f"found {len(blk64)} blk.64.* MTP layer tensors")

    ow = tensors["output.weight"]
    n_embd, n_vocab = ow.shape[0], ow.shape[1]
    print(f"output.weight: shape=({n_embd}, {n_vocab}) type={ow.tensor_type}")

    # selection
    if mode == "ext-d2t":
        if not ext_draft:
            print("ERROR: ext-d2t mode needs the external draft path (argv[5])")
            sys.exit(1)
        r_ext = GGUFReader(ext_draft)
        top_idx = np.array([t for t in r_ext.tensors if t.name == "d2t"][0].data,
                           dtype=np.int64)
        assert len(top_idx) == n_trim, f"external d2t has {len(top_idx)} entries, expected {n_trim}"
        print(f"ext-d2t mode: using external d2t token set ({len(top_idx)} tokens)")
    elif mode == "bpe":
        # low BPE ids (merge order ~ frequency) + all special/control tokens
        r_tok_type = reader.fields["tokenizer.ggml.token_type"].contents()
        tok_type = np.array(r_tok_type, dtype=np.int32)
        # types: 1=normal, 2=unknown, 3=control, 4=user-defined, 5=unused...
        special_ids = np.where((tok_type != 1) & (tok_type != 5))[0]
        n_special = len(special_ids)
        n_normal = n_trim - n_special
        normal_ids = np.arange(min(n_normal, n_vocab), dtype=np.int64)
        top_idx = np.concatenate([normal_ids, special_ids.astype(np.int64)])
        print(f"bpe mode: {n_normal} low-id normal + {n_special} special tokens")
    else:
        # l2 mode: selection happens after dequantize below
        pass

    # dequantize full head, then keep only selected rows
    # numpy layout is (n_vocab, n_embd), row j = token j's direction
    print("dequantizing output.weight to f16...")
    ow_f16 = dequantize(ow.data, GGMLQuantizationType(ow.tensor_type)).astype(np.float16)
    print(f"dequantized: shape={ow_f16.shape} dtype={ow_f16.dtype}")

    if mode == "l2":
        col_norms_sq = np.zeros(ow_f16.shape[0], dtype=np.float64)
        chunk = 4096
        for i in range(0, ow_f16.shape[0], chunk):
            block = ow_f16[i:i+chunk].astype(np.float32)
            col_norms_sq[i:i+chunk] = np.sum(block * block, axis=1)
            del block
        col_norms = np.sqrt(col_norms_sq)
        del col_norms_sq
        print(f"norms: min={col_norms.min():.4f} max={col_norms.max():.4f} mean={col_norms.mean():.4f}")
        top_idx = np.argsort(col_norms)[-n_trim:][::-1]
        print(f"selected top {n_trim}; norm range: {col_norms[top_idx[-1]]:.4f} .. {col_norms[top_idx[0]]:.4f}")
        del col_norms

    # trimmed head: [n_trim, n_embd] in f16 (token-major, matching source layout)
    ow_trimmed = ow_f16[top_idx].copy()
    del ow_f16  # free 2.5 GB

    # quantize trimmed head (gguf-py has no Q6_K quantizer; Q8_0 err ~0.017, Q4_0 ~0.35)
    qtype = GGMLQuantizationType[head_qname]
    print(f"quantizing trimmed head to {qtype.name}...")
    ow_trimmed_q = quantize(ow_trimmed.astype(np.float32), qtype)
    del ow_trimmed

    # d2t: maps trimmed column index -> original vocab token id
    d2t = top_idx.astype(np.int32)
    print(f"d2t: {len(d2t)} entries, first 8: {d2t[:8].tolist()}")

    # token_embd: copy as-is
    te = tensors["token_embd.weight"]
    print(f"token_embd.weight: shape={te.shape} type={te.tensor_type}")

    # --- write output GGUF ---
    print(f"writing {out_path}")
    writer = GGUFWriter(out_path, arch="qwen35")

    # copy metadata
    copy_metadata(reader, writer)

    # add tensor info (must precede write_header)
    on = tensors["output_norm.weight"]
    writer.add_tensor_info("token_embd.weight", te.data.shape, te.data.dtype, te.n_bytes, te.tensor_type)
    writer.add_tensor_info("output_norm.weight", on.data.shape, on.data.dtype, on.n_bytes, on.tensor_type)
    writer.add_tensor_info("output.weight", ow_trimmed_q.shape, ow_trimmed_q.dtype, ow_trimmed_q.nbytes, qtype)
    writer.add_tensor_info("d2t", d2t.shape, d2t.dtype, d2t.nbytes)
    for t in blk64:
        writer.add_tensor_info(t.name, t.data.shape, t.data.dtype, t.n_bytes, t.tensor_type)

    # write header + kv + tensor info
    writer.write_header_to_file()
    writer.write_kv_data_to_file()
    writer.write_ti_data_to_file()

    # write tensor data
    print("writing tensor data...")
    writer.write_tensor_data(te.data)
    writer.write_tensor_data(on.data)
    writer.write_tensor_data(ow_trimmed_q)
    writer.write_tensor_data(d2t)
    for t in blk64:
        writer.write_tensor_data(t.data)

    writer.close()
    size = os.path.getsize(out_path)
    print(f"done: {out_path} ({size/1e6:.1f} MB)")


if __name__ == "__main__":
    main()
