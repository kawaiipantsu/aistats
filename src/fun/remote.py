"""Read-only SSH log exporter; sent on stdin to Python 3, never installed.

Protocol: bounded JSON headers followed by exactly `size` bytes for a file.
Only session/history logs are opened; no credentials or tool settings are read.
"""
import errno
import json
import os
import pwd
import stat
import sys

MAX_DEPTH = 6
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK


def header(value):
    sys.stdout.buffer.write(json.dumps(value, ensure_ascii=True).encode() + b"\n")


def open_directory(path, trusted):
    if trusted:
        return os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    # Check every component, not just the last one. A different user's
    # .claude directory must not redirect a root scan into someone else's home.
    fd = os.open("/", DIRECTORY_FLAGS)
    try:
        for part in os.path.abspath(path).split("/"):
            if part:
                child = os.open(part, DIRECTORY_FLAGS, dir_fd=fd)
                os.close(fd)
                fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def walk(directory, depth=0):
    if depth > MAX_DEPTH:
        return
    with os.scandir(directory) as entries:
        for entry in entries:
            if entry.is_dir(follow_symlinks=False):
                try:
                    child = os.open(entry.name, DIRECTORY_FLAGS, dir_fd=directory)
                except OSError:
                    continue
                try:
                    yield from walk(child, depth + 1)
                finally:
                    os.close(child)
            elif entry.is_file(follow_symlinks=False) and entry.name.endswith(".jsonl"):
                yield directory, entry.name


def accounts():
    own = pwd.getpwuid(os.geteuid())
    entries = pwd.getpwall() if os.geteuid() == 0 else [own]
    users = [(entry.pw_name, entry.pw_dir) for entry in entries]
    if os.geteuid() == 0:
        try:
            with os.scandir("/home") as homes:
                users.extend((e.name, e.path) for e in homes if e.is_dir(follow_symlinks=False))
        except OSError:
            pass
    seen = set()
    for name, home in users:
        if home.startswith("/") and home != "/" and home not in seen:
            seen.add(home)
            yield name, home, home == own.pw_dir


class Exporter:
    def __init__(self):
        self.unreadable = 0
        self.seen = set()

    def file(self, directory, name, kind):
        try:
            fd = os.open(name, FILE_FLAGS, dir_fd=directory)
        except OSError as error:
            if error.errno not in (errno.ENOENT, errno.ELOOP):
                self.unreadable += 1
            return False
        with os.fdopen(fd, "rb") as source:
            info = os.fstat(source.fileno())
            if not stat.S_ISREG(info.st_mode):
                return False
            identity = (info.st_dev, info.st_ino)
            if identity in self.seen:
                return True
            self.seen.add(identity)
            header({"type": "file", "kind": kind, "name": name, "size": info.st_size})
            remaining = info.st_size
            while remaining:
                chunk = source.read(min(remaining, 1024 * 1024))
                if not chunk:
                    raise RuntimeError("a log was truncated during collection; retry")
                sys.stdout.buffer.write(chunk)
                remaining -= len(chunk)
        return True

    def user(self, name, home, trusted):
        header({"type": "user", "name": name, "home": home})
        self.seen.clear()
        claude = [os.path.join(home, ".claude"), os.path.join(home, ".config/claude")]
        codex = [os.path.join(home, ".codex")]
        if trusted:
            if os.environ.get("CLAUDE_CONFIG_DIR"):
                claude.append(os.environ["CLAUDE_CONFIG_DIR"])
            if os.environ.get("CODEX_HOME"):
                codex.append(os.environ["CODEX_HOME"])
        for tool, roots, folders in [("claude", claude, ["projects"]),
                                      ("codex", codex, ["sessions", "archived_sessions"])]:
            history_sent = False
            for root in dict.fromkeys(roots):
                try:
                    directory = open_directory(root, trusted)
                except OSError:
                    continue
                try:
                    for folder in folders:
                        try:
                            sessions = os.open(folder, DIRECTORY_FLAGS, dir_fd=directory)
                        except OSError:
                            continue
                        try:
                            for parent, filename in walk(sessions):
                                self.file(parent, filename, tool + "_session")
                        finally:
                            os.close(sessions)
                    if not history_sent:
                        history_sent = self.file(directory, "history.jsonl", tool + "_history")
                finally:
                    os.close(directory)


def main():
    header({"type": "hello", "version": 1, "all_users": os.geteuid() == 0})
    exporter = Exporter()
    for user in accounts():
        exporter.user(*user)
    header({"type": "done", "unreadable": exporter.unreadable})
    sys.stdout.buffer.flush()


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Never print a traceback containing a private path or log content.
        sys.stderr.write("ai-stats: remote log export failed; check access and retry\n")
        sys.exit(1)
