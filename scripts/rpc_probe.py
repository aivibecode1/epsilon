#!/usr/bin/env python3
"""Drive `epsilon --rpc` from a script and report what happened.

Why this exists: the unit suite runs against the faux provider in-process, so it
cannot catch a broken *frontend* — an event that is never emitted, an approval
that never resolves, a run that ends without reaching the client. This script is
the smallest honest end-to-end check of the RPC surface, and it doubles as an
executable description of the protocol.

Usage:
    python scripts/rpc_probe.py [--exe target/debug/epsilon.exe] [--script FILE]
                                [--workdir DIR] [--timeout SECS]

Protocol (see crates/epsilon-cli/src/rpc_mode.rs):
    in : {"op":"prompt"|"steer"|"followup"|"approvals"|"approve"|"ping", ...}
    out: {"ok":{...}} or {"error":"..."} replies, plus {"event":{...}} frames

Exit code: 0 when the run reached agent_end with at least one tool execution,
1 otherwise. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class Probe:
    def __init__(self, exe: str, script: str | None, workdir: str, timeout: float):
        self.timeout = timeout
        args = [exe, "--rpc", "--sandbox", "off"]
        if script:
            args += ["--script", script]
        self.proc = subprocess.Popen(
            args,
            cwd=workdir,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        self.events: list[dict] = []
        self.replies: list[dict] = []
        self._lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                frame = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "event" in frame:
                self.events.append(frame["event"])
            else:
                self.replies.append(frame)

    def send(self, obj: dict) -> None:
        assert self.proc.stdin is not None
        with self._lock:
            self.proc.stdin.write(json.dumps(obj) + "\n")
            self.proc.stdin.flush()

    def pending(self) -> list[dict]:
        """Latest `approvals` reply's pending list (empty when none)."""
        for reply in reversed(self.replies):
            ok = reply.get("ok")
            if isinstance(ok, dict) and "pending" in ok:
                return ok["pending"]
        return []

    def run(self, task: str) -> dict:
        self.send({"op": "prompt", "task": task})
        approved = 0
        deadline = time.time() + self.timeout
        while time.time() < deadline:
            self.send({"op": "approvals"})
            time.sleep(0.3)
            for ask in self.pending():
                self.send(
                    {
                        "op": "approve",
                        "id": ask.get("request_id") or ask.get("id"),
                        "decision": {"type": "allow_once"},
                    }
                )
                approved += 1
            if any(e.get("type") == "agent_end" for e in self.events):
                break
        self.proc.stdin.close()  # type: ignore[union-attr]
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        return {"approved": approved}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=os.path.join(REPO, "target", "debug", "epsilon.exe"))
    ap.add_argument("--script", default=None, help="faux-provider script JSON")
    ap.add_argument("--workdir", default=None, help="workspace dir (default: a temp dir)")
    ap.add_argument("--timeout", type=float, default=30.0)
    args = ap.parse_args()

    workdir = args.workdir
    if workdir is None:
        import tempfile

        workdir = tempfile.mkdtemp(prefix="epsilon-rpc-probe-")
    if not os.path.exists(args.exe):
        print(f"no such binary: {args.exe} (build with: cargo build -p epsilon-cli)")
        return 2

    probe = Probe(args.exe, args.script, workdir, args.timeout)
    started = probe.run("reply with the single word ok")
    kinds: dict[str, int] = {}
    for ev in probe.events:
        kinds[ev.get("type", "?")] = kinds.get(ev.get("type", "?"), 0) + 1

    print(f"workdir: {workdir}")
    print(f"approvals answered: {started['approved']}")
    print("event kinds:", json.dumps(kinds, sort_keys=True))
    updates = [e for e in probe.events if e.get("type") == "tool_execution_update"]
    for ev in updates[:3]:
        text = "".join(
            block.get("text", "")
            for block in ev.get("partial", {}).get("content", [])
            if isinstance(block, dict)
        )
        print("  live update tail:", repr(text[-40:]))
    print("live tool updates:", len(updates))

    ok = kinds.get("agent_end", 0) >= 1 and kinds.get("agent_start", 0) >= 1
    print("RESULT:", "ok" if ok else "run did not reach agent_end")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
