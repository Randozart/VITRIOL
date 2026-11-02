"""VITRIOL Socket Client"""

import socket
import json
import time
from typing import Optional, Dict, Any


class VitriolError(Exception):
    """VITRIOL error exception"""
    def __init__(self, code: str, message: str):
        self.code = code
        self.message = message
        super().__init__(f"[{code}] {message}")


class VitriolClient:
    """Client for VITRIOL daemon socket API"""

    DEFAULT_SOCKET = "/var/run/vitriol.sock"
    DEFAULT_TIMEOUT = 30.0

    def __init__(self, socket_path: str = DEFAULT_SOCKET, timeout: float = DEFAULT_TIMEOUT):
        self.socket_path = socket_path
        self.timeout = timeout
        self._sock: Optional[socket.socket] = None
        self._request_id = 0

    def connect(self) -> None:
        """Connect to VITRIOL daemon"""
        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._sock.settimeout(self.timeout)
        self._sock.connect(self.socket_path)

    def close(self) -> None:
        """Close connection"""
        if self._sock:
            self._sock.close()
            self._sock = None

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()
        return False

    def _send_request(self, cmd: str, params: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Send JSON request and receive response"""
        if not self._sock:
            raise VitriolError("NOT_CONNECTED", "Not connected to daemon")

        self._request_id += 1
        request = {
            "cmd": cmd,
            "params": params or {},
            "id": self._request_id
        }

        # Send length-prefixed JSON
        data = json.dumps(request).encode('utf-8')
        header = len(data).to_bytes(4, 'little')
        self._sock.sendall(header + data)

        # Receive length prefix
        header = b''
        while len(header) < 4:
            header += self._sock.recv(4 - len(header))
        body_len = int.from_bytes(header, 'little')

        # Receive body
        body = b''
        while len(body) < body_len:
            body += self._sock.recv(body_len - len(body))

        response = json.loads(body.decode('utf-8'))

        if response.get("id") != self._request_id:
            raise VitriolError("PROTOCOL_ERROR", f"ID mismatch: expected {self._request_id}, got {response.get('id')}")

        return response

    def status(self) -> Dict[str, Any]:
        """Get VITRIOL status"""
        return self._send_request("STATUS")

    def load_model(self, path: str) -> Dict[str, Any]:
        """Load model from file path"""
        return self._send_request("LOAD_MODEL", {"path": path})

    def infer(self, prompt: str, max_tokens: int = 100, temperature: float = 0.7) -> Dict[str, Any]:
        """Run inference"""
        return self._send_request("INFER", {
            "prompt": prompt,
            "max_tokens": max_tokens,
            "temperature": temperature
        })

    def stream_layer(self, layer_id: int, ssd_offset: int, size: int) -> Dict[str, Any]:
        """Stream layer from SSD to GPU"""
        return self._send_request("STREAM_LAYER", {
            "layer_id": layer_id,
            "ssd_offset": ssd_offset,
            "size": size
        })

    def evict_layer(self, layer_id: int) -> Dict[str, Any]:
        """Evict layer from VRAM (LRU)"""
        return self._send_request("EVICT_LAYER", {"layer_id": layer_id})

    def set_safety(self, level: int) -> Dict[str, Any]:
        """Set safety level (1=safe, 2=dma, 3=raw pci)"""
        return self._send_request("SET_SAFETY", {"level": level})

    def get_status(self) -> Dict[str, Any]:
        """Get typed status"""
        resp = self.status()
        if resp["status"] != "ok":
            raise VitriolError(resp.get("error", {}).get("code", "UNKNOWN"),
                             resp.get("error", {}).get("message", "Unknown error"))
        return resp["data"]


if __name__ == "__main__":
    import sys

    # Example usage
    if len(sys.argv) < 2:
        print("Usage: python client.py <command> [args...]")
        print("Commands: status, infer <prompt>")
        sys.exit(1)

    cmd = sys.argv[1]

    try:
        with VitriolClient() as client:
            if cmd == "status":
                status = client.get_status()
                print(json.dumps(status, indent=2))
            elif cmd == "infer":
                if len(sys.argv) < 3:
                    print("Usage: client.py infer <prompt>")
                    sys.exit(1)
                result = client.infer(sys.argv[2])
                print(f"Output: {result['data']['output']}")
            else:
                print(f"Unknown command: {cmd}")
                sys.exit(1)
    except VitriolError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)
