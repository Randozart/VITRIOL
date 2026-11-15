"""Optional CUDA-Python functional emulator; no GPU timing or compiled-CUDA claim.

PROVENANCE: inspiration — none; original small algorithm checks, not ggml kernels.
"""
import json
import math
import os
import sys

cuda = None

def matvec(matrix, vector, output, cols):
    row = cuda.grid(1)
    if row < output.size:
        value = 0.0
        for col in range(cols):
            value += matrix[row * cols + col] * vector[col]
        output[row] = value

def dot(left, right, output):
    if cuda.grid(1) == 0:
        value = 0.0
        for index in range(left.size):
            value += left[index] * right[index]
        output[0] = value

def softmax(values, output):
    if cuda.grid(1) == 0:
        maximum = values[0]
        for index in range(1, values.size):
            maximum = max(maximum, values[index])
        total = 0.0
        for index in range(values.size):
            total += math.exp(values[index] - maximum)
        for index in range(values.size):
            output[index] = math.exp(values[index] - maximum) / total


def main():
    if os.environ.get("NUMBA_ENABLE_CUDASIM") != "1":
        print("NUMBA_ENABLE_CUDASIM=1 is required; this adapter never selects hardware", file=sys.stderr)
        return 77
    try:
        import numba
        import numpy as np
        from numba import cuda
    except ImportError as error:
        print(f"optional emulator unavailable: {error}", file=sys.stderr)
        return 77

    # Numba's simulator replaces CUDA symbols in kernel globals.
    globals()["cuda"] = cuda

    matvec_kernel = cuda.jit(matvec)
    dot_kernel = cuda.jit(dot)
    softmax_kernel = cuda.jit(softmax)

    job = json.load(sys.stdin)
    def array(values):
        return np.asarray(values, dtype=np.float64)
    operation = job["op"]
    if operation == "matvec":
        output = np.zeros(job["rows"], dtype=np.float64)
        matvec_kernel[(job["rows"] + 31) // 32, 32](array(job["matrix"]), array(job["vector"]), output, job["cols"])
    elif operation == "dot":
        output = np.zeros(1, dtype=np.float64)
        dot_kernel[1, 1](array(job["left"]), array(job["right"]), output)
    elif operation == "softmax":
        output = np.zeros(len(job["values"]), dtype=np.float64)
        softmax_kernel[1, 1](array(job["values"]), output)
    else:
        print(f"unsupported operation: {operation}", file=sys.stderr)
        return 77
    print(json.dumps({"version": 1, "output": output.tolist(), "backend": f"numba {numba.__version__} CUDA simulator"}, allow_nan=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
