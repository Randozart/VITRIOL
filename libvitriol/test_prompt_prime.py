# 2026-11-18 14:02 — Model-free cache priming invariants and failure admission.
import copy
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from libvitriol.prompt_prime import EngineHTTP, main, prime, validate_url


class Transport:
    def __init__(self, mutation=None):
        self.calls = []
        self.mutation = mutation
        self.ready_called = False

    def ready(self):
        self.ready_called = True

    def request(self, route, body=None):
        self.calls.append((route, copy.deepcopy(body)))
        if route == "/apply-template":
            return {"prompt": body["messages"][-1]["content"]}
        if route == "/tokenize":
            return {"tokens": [1, 2, 3 if body["content"] == "!" else 4, 9]}
        result = {
            "stop": True,
            "truncated": False,
            "timings": {"cache_n": 0, "prompt_n": 2, "predicted_n": 1},
            "content": "discard me",
        }
        if self.mutation:
            self.mutation(result)
        return result


class PrimeTests(unittest.TestCase):
    def test_actual_template_prefix_only_and_no_input_mutation(self):
        messages = [{"role": "system", "content": "original"}]
        snapshot = copy.deepcopy(messages)
        client = Transport()
        result = prime(
            "http://127.0.0.1:8082", messages, "same-model", transport=client
        )
        self.assertEqual(messages, snapshot)
        self.assertTrue(client.ready_called)
        self.assertEqual(
            client.calls[-1],
            (
                "/completion",
                {
                    "prompt": [1, 2],
                    "n_predict": 1,
                    "temperature": 0,
                    "cache_prompt": True,
                    "stream": False,
                },
            ),
        )
        self.assertEqual(result["prefix_tokens"], 2)
        self.assertNotIn("content", result)
        self.assertNotIn("discard me", json.dumps(result))

    def test_no_false_ready_when_inference_truncated_or_counters_missing(self):
        for key, value in [
            ("truncated", True),
            ("stop", False),
            ("timings", {}),
            ("timings", {"cache_n": 0, "prompt_n": 1, "predicted_n": 1}),
            ("timings", {"cache_n": 0, "prompt_n": 2, "predicted_n": 0}),
        ]:
            with (
                self.subTest(key=key, value=value),
                self.assertRaisesRegex(ValueError, "not admitted"),
            ):
                prime(
                    "http://127.0.0.1:8082",
                    [{"role": "system", "content": "x"}],
                    transport=Transport(
                        lambda r, key=key, value=value: r.update({key: value})
                    ),
                )

    def test_cached_prefix_still_requires_real_inference(self):
        client = Transport(lambda r: r["timings"].update(cache_n=1, prompt_n=1))
        result = prime(
            "http://127.0.0.1:8082",
            [{"role": "system", "content": "x"}],
            transport=client,
        )
        self.assertEqual(result["timings"]["cache_n"], 1)
        self.assertEqual(result["sampled_tokens_discarded"], 1)

    def test_identical_or_disjoint_probes_are_not_admitted(self):
        for tokens in ([1, 2], []):
            client = Transport()
            request = client.request
            client.request = lambda route, body=None, tokens=tokens, request=request: (
                {"tokens": tokens} if route == "/tokenize" else request(route, body)
            )
            with self.subTest(tokens=tokens), self.assertRaises(ValueError):
                prime(
                    "http://127.0.0.1:8082",
                    [{"role": "system", "content": "x"}],
                    transport=client,
                )

    def test_invalid_input_refused_before_http(self):
        for messages in (
            [],
            None,
            [{"content": "x"}],
            [{"role": "system", "content": []}],
        ):
            client = Transport()
            with self.subTest(messages=messages), self.assertRaises(ValueError):
                prime("http://127.0.0.1:8082", messages, transport=client)
            self.assertFalse(client.ready_called)

    def test_deadline_prevents_even_first_network_call(self):
        with patch("libvitriol.prompt_prime.time.monotonic", side_effect=[0, 2]):
            client = EngineHTTP("http://127.0.0.1:8082", 1)
            with (
                patch.object(client.opener, "open") as call,
                self.assertRaises(TimeoutError),
            ):
                client.request("/health")
            call.assert_not_called()

    def test_no_success_for_response_arriving_after_total_deadline(self):
        response = io.StringIO('{"status":"ok"}')
        with patch("libvitriol.prompt_prime.time.monotonic", side_effect=[0, 0.1, 2]):
            client = EngineHTTP("http://127.0.0.1:8082", 1)
            with (
                patch.object(client.opener, "open", return_value=response),
                self.assertRaises(TimeoutError),
            ):
                client.request("/health")

    def test_timeout_domain_and_credential_urls(self):
        for value in (0, -1, float("nan"), float("inf"), 601):
            with self.subTest(value=value), self.assertRaises(ValueError):
                EngineHTTP("http://127.0.0.1:8082", value)
        for value in (
            "file:///tmp/model",
            "http://user:secret@localhost:8080",
            "http://localhost?a=b",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_url(value)

    def test_cli_failure_cannot_publish_success(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory)
            (p / "messages.json").write_text('[{"role":"system","content":"x"}]')
            args = [
                "vitriol-prime",
                "--url",
                "http://localhost:8082",
                "--messages-file",
                str(p / "messages.json"),
                "--output",
                str(p / "ready.json"),
            ]
            with (
                patch("sys.argv", args),
                patch(
                    "libvitriol.prompt_prime.prime",
                    side_effect=TimeoutError("deadline"),
                ),
                patch("sys.stdout", io.StringIO()),
                patch("sys.stderr", io.StringIO()),
            ):
                self.assertEqual(main(), 1)
            self.assertFalse((p / "ready.json").exists())


if __name__ == "__main__":
    unittest.main()
