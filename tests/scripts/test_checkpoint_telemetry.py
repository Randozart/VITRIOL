"""Checkpoint observations must not invent transfer time or change save behaviour."""
import importlib.util
import io
import json
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "slot_persist", Path(__file__).resolve().parents[2] / "scripts/lull_slot_persist.py"
)
persist = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(persist)


class CheckpointTelemetryTest(unittest.TestCase):
    def test_engine_discovery_matches_the_sidecar_port(self):
        def read_process(path, *_args):
            if path.endswith("/comm"):
                return io.StringIO("llama-server\n")
            port = "18781" if "/80/" in path else str(persist.PORT)
            return io.BytesIO(f"llama-server\0--port\0{port}\0".encode())

        with patch.object(persist.os, "listdir", return_value=["80", "81"]), patch(
            "builtins.open", side_effect=read_process
        ):
            self.assertEqual(persist.server_pid(), 81)

    def test_existing_api_values_are_reported_without_extra_requests(self):
        for operation, key in [("save", "n_written"), ("restore", "n_read")]:
            with patch.object(persist, "server_pid", return_value=81), patch.object(
                persist, "log"
            ) as log, patch.object(persist, "post") as post:
                persist.log_checkpoint_io(operation, 0, {
                    key: 1234, "timings": {f"{operation}_ms": 0.0}
                }, 81)
                event = json.loads(log.call_args.args[0].split(": ", 1)[1])
                self.assertEqual(event["bytes"], 1234)
                self.assertEqual(event["duration_ms"], 0.0)
                self.assertEqual(event["pid"], 81)
                post.assert_not_called()

    def test_unknown_pending_and_changed_engine_are_not_attributed(self):
        for response in [{}, {"timings": None}, {"pending": True},
                         {"n_written": 1, "timings": {"save_ms": float("nan")}},
                         {"n_written": -1, "timings": {"save_ms": 2}}]:
            with patch.object(persist, "server_pid", return_value=81), patch.object(
                persist, "log"
            ) as log:
                persist.log_checkpoint_io("save", 0, response, 81)
                log.assert_not_called()
        with patch.object(persist, "server_pid", return_value=82), patch.object(
            persist, "log"
        ) as log:
            persist.log_checkpoint_io("save", 0, {
                "n_written": 1, "timings": {"save_ms": 1}
            }, 81)
            log.assert_not_called()


if __name__ == "__main__":
    unittest.main()
