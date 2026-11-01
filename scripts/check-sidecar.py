#!/usr/bin/env python3
"""Smoke-test the actual compiled sidecar with an isolated, empty session."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="line-gtk-smoke-") as directory:
    env = {**os.environ, "HOME": directory, "DENO_DIR": directory + "/deno",
           "LINE_GTK_DATA": directory + "/data"}
    try:
        result = subprocess.run([str(binary)], input='{"id":1,"method":"ping"}\n',
                                text=True, capture_output=True, env=env, cwd=directory,
                                timeout=15)
        rows = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
        ready = any(row.get("event") == "ready" and not row.get("hasAuth") for row in rows)
        pong = any(row.get("id") == 1 and row.get("ok") is True
                   and row.get("result", {}).get("pong") is True for row in rows)
        if result.returncode or not ready or not pong:
            raise RuntimeError("sidecar did not complete ready + ping\n" + result.stderr)
    except (subprocess.TimeoutExpired, RuntimeError, ValueError, OSError) as error:
        raise SystemExit(f"Sidecar smoke test failed ({binary}): {error}") from error
print(f"Sidecar ready + ping passed: {binary}")
