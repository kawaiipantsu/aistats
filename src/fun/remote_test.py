"""Run with python3 -B -m unittest discover -s src/fun -p remote_test.py."""
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import remote


def decode(data):
    stream = io.BytesIO(data)
    records = []
    while stream.tell() < len(data):
        record = json.loads(stream.readline())
        if record["type"] == "file":
            record["content"] = stream.read(record["size"])
        records.append(record)
    return records


class ExportTests(unittest.TestCase):
    def test_real_discovery_skips_links_special_files_and_secrets(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp) / "user"
            claude = home / ".claude/projects/project"
            codex = home / ".codex/archived_sessions"
            claude.mkdir(parents=True)
            codex.mkdir(parents=True)
            (claude / "session.jsonl").write_bytes(b'{"prompt":"private text"}\n')
            (codex / "archived.jsonl").write_bytes(b'{}\n')
            (home / ".codex/history.jsonl").write_bytes(b'{}\n')
            (home / ".codex/auth.json").write_text("secret credentials")
            (home / ".codex/config.toml").write_text("secret config")
            (claude / "link.jsonl").symlink_to(home / ".codex/auth.json")
            (claude / "loop").symlink_to(claude, target_is_directory=True)
            os.mkfifo(claude / "pipe.jsonl")
            output = io.TextIOWrapper(io.BytesIO())
            with contextlib.redirect_stdout(output):
                exporter = remote.Exporter()
                exporter.user("alice", str(home), False)
            records = decode(output.buffer.getvalue())
            self.assertEqual({r["name"] for r in records if r["type"] == "file"},
                             {"session.jsonl", "archived.jsonl", "history.jsonl"})
            self.assertNotIn(b"secret credentials", output.buffer.getvalue())
            self.assertEqual(exporter.unreadable, 0)

    def test_other_users_cannot_redirect_log_roots(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp) / "user"
            target = Path(tmp) / "victim"
            home.mkdir()
            (target / "projects").mkdir(parents=True)
            (target / "projects/stolen.jsonl").write_text("private")
            (home / ".claude").symlink_to(target, target_is_directory=True)
            output = io.TextIOWrapper(io.BytesIO())
            with contextlib.redirect_stdout(output):
                remote.Exporter().user("alice", str(home), False)
            self.assertEqual(len(decode(output.buffer.getvalue())), 1)

    def test_nonroot_discovery_does_not_enumerate_other_accounts(self):
        entry = type("Entry", (), {"pw_name": "alice", "pw_dir": "/home/alice"})()
        with patch.object(remote.os, "geteuid", return_value=1000), \
             patch.object(remote.pwd, "getpwuid", return_value=entry), \
             patch.object(remote.pwd, "getpwall", side_effect=AssertionError("enumerated users")):
            self.assertEqual(list(remote.accounts()), [("alice", "/home/alice", True)])


if __name__ == "__main__":
    unittest.main()
