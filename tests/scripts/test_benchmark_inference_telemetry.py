"""Protocol and attribution checks for the isolated telemetry benchmark."""
import importlib.util
import io
import json
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "inference_benchmark", Path(__file__).resolve().parents[2] / "scripts/benchmark_inference_telemetry.py"
)
benchmark = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(benchmark)


class InferenceTelemetryTest(unittest.TestCase):
    def test_stream_ignores_empty_role_and_keepalive_events(self):
        chunks = [": keepalive\n", 'data: {"content":""}\n',
                  'data: {"choices":[{"delta":{"role":"assistant"}}]}\n',
                  'data: {"content":"first"}\n',
                  'data: {"stop":true,"timings":{"predicted_n":128,"predicted_ms":1000}}\n']
        stream = io.BytesIO("".join(chunks).encode())
        with patch.object(benchmark, "post", return_value=stream), patch.object(
            benchmark.time, "perf_counter", side_effect=[10, 10.5, 12]
        ):
            sample = benchmark.stream_sample("http://127.0.0.1:18781", 128)
        self.assertEqual(sample["client_first_content_s"], .5)
        self.assertEqual(sample["wall_s"], 2)
        self.assertEqual(sample["decode_tps"], 128)

    def test_incomplete_stream_fails_instead_of_reporting_zero(self):
        for events in [[{"content": ""}], [{"content": "a"}], [{"error": "outage"}],
                       [{"content": "a"}, {"stop": True, "timings": {
                           "predicted_n": 128, "predicted_ms": float("nan")}}]]:
            stream = io.BytesIO("".join(f"data: {json.dumps(e)}\n" for e in events).encode())
            with patch.object(benchmark, "post", return_value=stream), self.assertRaises(RuntimeError):
                benchmark.stream_sample("http://127.0.0.1:18781", 128)

    def test_production_or_remote_endpoints_are_rejected(self):
        for target in ["http://127.0.0.1:8279", "http://example.com:8080", "http://127.0.0.1:8080/other"]:
            with self.assertRaises(ValueError):
                benchmark.validate_target(target)
        self.assertEqual(benchmark.validate_target("http://127.0.0.1:18781"), 18781)

    def test_paired_regression_is_not_accepted(self):
        pairs = [{"off": {"decode_tps": 100}, "on": {"decode_tps": 90}}] * 6
        summary = benchmark.overhead_summary(pairs)
        self.assertFalse(summary["below_one_percent"])
        self.assertGreater(summary["bootstrap_95_percent_interval"][0], .01)


if __name__ == "__main__":
    unittest.main()
