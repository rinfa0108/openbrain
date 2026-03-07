from __future__ import annotations

import json
import queue
import subprocess
import threading
import time
from dataclasses import dataclass, field
from typing import Any, Optional, TypeAlias

from .error import OpenBrainError

RpcRequest: TypeAlias = dict[str, Any]
RpcResponse: TypeAlias = dict[str, Any]


@dataclass
class OpenBrainMcpClient:
    proc: subprocess.Popen
    timeout_sec: float = 10.0
    _responses: "queue.Queue[tuple[Optional[int], RpcResponse]]" = field(default_factory=queue.Queue)
    _next_id: int = 1
    _closed: bool = False

    @classmethod
    def spawn_openbrain_mcp(
        cls,
        openbrain_path: str = "openbrain",
        args: Optional[list[str]] = None,
        env: Optional[dict[str, str]] = None,
        cwd: Optional[str] = None,
        startup_timeout_sec: float = 5.0,
    ) -> "OpenBrainMcpClient":
        proc = subprocess.Popen(
            [openbrain_path, *(args or [])],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            cwd=cwd,
        )
        if proc.stdin is None or proc.stdout is None:
            raise OpenBrainError("MCP_ERROR", "failed to create stdio pipes")

        client = cls(proc=proc)
        threading.Thread(target=cls._reader_loop, args=(proc, client._responses), daemon=True).start()
        start = time.time()
        while proc.poll() is None and time.time() - start < startup_timeout_sec:
            time.sleep(0.05)
        if proc.poll() is not None:
            raise OpenBrainError("MCP_ERROR", "MCP process exited during startup")
        return client

    @staticmethod
    def _reader_loop(proc: subprocess.Popen, q: "queue.Queue[tuple[Optional[int], RpcResponse]]") -> None:
        if proc.stdout is None:
            return
        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            msg_id = payload.get("id")
            q.put((msg_id, payload))

    def call_tool(self, tool: str, args: dict[str, Any], timeout_sec: Optional[float] = None) -> Any:
        if self._closed:
            raise OpenBrainError("MCP_CLOSED", "MCP client is closed")

        if self.proc.stdin is None:
            raise OpenBrainError("MCP_ERROR", "stdin is unavailable")

        msg_id = self._next_id
        self._next_id += 1
        req: RpcRequest = {
            "jsonrpc": "2.0",
            "id": msg_id,
            "method": "tools/call",
            "params": {"name": tool, "arguments": args},
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()

        target_timeout = timeout_sec if timeout_sec is not None else self.timeout_sec
        try:
            while True:
                response_id, payload = self._responses.get(timeout=target_timeout)
                if response_id == msg_id:
                    break
        except queue.Empty as err:
            raise OpenBrainError("TIMEOUT", "MCP call timed out") from err

        if not isinstance(payload, dict):
            raise OpenBrainError("MCP_ERROR", "invalid MCP payload")
        if "error" in payload:
            raise OpenBrainError("MCP_ERROR", payload["error"].get("message", "unknown"), details=payload["error"])
        result = payload.get("result")
        if not isinstance(result, dict):
            raise OpenBrainError("MCP_ERROR", "missing result payload")
        if result.get("ok") is False:
            error = result.get("error")
            if not isinstance(error, dict):
                raise OpenBrainError("OB_INTERNAL", "invalid error payload", details=result)
            raise OpenBrainError(error.get("code", "OB_INTERNAL"), error.get("message", "error"), details=error.get("details"))

        return result

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self.proc.stdin:
            self.proc.stdin.close()
        self.proc.terminate()
        try:
            self.proc.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            self.proc.kill()
