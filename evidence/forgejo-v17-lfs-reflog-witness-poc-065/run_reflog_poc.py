from pathlib import Path
import os
import shlex
import subprocess
import sys

root = Path(__file__).resolve().parent
scratch = root / "scratch"
config = scratch / "forgejo-internal.gitconfig"
env = os.environ.copy()
env["GIT_CONFIG_GLOBAL"] = str(config)
env["GIT_CONFIG_NOSYSTEM"] = "1"
env["GIT_TERMINAL_PROMPT"] = "0"
env["GIT_PAGER"] = "cat"
env["GIT_AUTHOR_NAME"] = "Forgejo LFS recovery POC"
env["GIT_AUTHOR_EMAIL"] = "poc@example.invalid"
env["GIT_COMMITTER_NAME"] = env["GIT_AUTHOR_NAME"]
env["GIT_COMMITTER_EMAIL"] = env["GIT_AUTHOR_EMAIL"]
env["GIT_AUTHOR_DATE"] = "2026-10-25T12:00:00Z"
env["GIT_COMMITTER_DATE"] = "2026-10-25T12:00:00Z"
log = []
commands = []

if scratch.exists():
    result_path = root / "result.txt"
    if not result_path.is_file() or not config.is_file():
        raise SystemExit("Existing partial scratch preserved; refusing to reinitialize or overwrite it")
    saved = dict(line.split("=", 1) for line in result_path.read_text().splitlines() if "=" in line)
    bare = scratch / "server.git"
    checks = [
        (["--git-dir", bare, "rev-parse", "refs/heads/main"], 0, saved["base_commit"]),
        (["--git-dir", bare, "config", "--get", "core.logAllRefUpdates"], 0, "true"),
        (["--git-dir", bare, "config", "--get", "gc.reflogExpire"], 0, "90"),
        (["--git-dir", bare, "reflog", "show", "--format=%H%x09%gs", "refs/heads/main"], 0, saved["candidate_commit"]),
        (["--git-dir", bare, "cat-file", "-e", f"{saved['candidate_commit']}^{{commit}}"], 0, ""),
        (["--git-dir", bare, "merge-base", "--is-ancestor", saved["candidate_commit"], "refs/heads/main"], 1, ""),
    ]
    for args, expected, contains in checks:
        completed = subprocess.run(["git", *map(str, args)], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if completed.returncode != expected or (contains and contains not in completed.stdout):
            raise SystemExit(f"Read-only recheck failed; existing scratch preserved: git {shlex.join(map(str, args))}")
    print("PASS existing reflog evidence revalidated read-only; no scratch files changed")
    sys.exit(0)

scratch.mkdir()

def run(args, cwd=None, expected=0):
    cmd = ["git", *map(str, args)]
    commands.append(shlex.join(cmd))
    result = subprocess.run(cmd, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log.append(f"$ {shlex.join(cmd)}\n{result.stdout}exit={result.returncode}\n")
    if result.returncode != expected:
        raise RuntimeError(f"command exited {result.returncode}, expected {expected}: {shlex.join(cmd)}")
    return result.stdout.strip()

try:
    version = run(["--version"])
    run(["config", "--global", "core.logAllRefUpdates", "true"])
    run(["config", "--global", "gc.reflogExpire", "90"])
    bare = scratch / "server.git"
    work = scratch / "work"
    run(["init", "--bare", "--template=", bare])
    run(["init", "--template=", "-b", "main", work])
    run(["-C", work, "config", "user.name", "Forgejo LFS recovery POC"])
    run(["-C", work, "config", "user.email", "poc@example.invalid"])
    (work / "payload.txt").write_text("base\n")
    run(["-C", work, "add", "payload.txt"])
    run(["-C", work, "commit", "-m", "base"])
    base = run(["-C", work, "rev-parse", "HEAD"])
    run(["-C", work, "remote", "add", "origin", bare])
    run(["-C", work, "push", "origin", "HEAD:refs/heads/main"])
    (work / "payload.txt").write_text("accepted candidate\n")
    run(["-C", work, "commit", "-am", "candidate"])
    candidate = run(["-C", work, "rev-parse", "HEAD"])
    run(["-C", work, "push", "origin", "HEAD:refs/heads/main"])
    run(["-C", work, "reset", "--hard", base])
    run(["-C", work, "push", "--force", "origin", "HEAD:refs/heads/main"])
    current = run(["--git-dir", bare, "rev-parse", "refs/heads/main"])
    reflog = run(["--git-dir", bare, "reflog", "show", "--format=%H%x09%gs", "refs/heads/main"])
    run(["--git-dir", bare, "config", "--show-origin", "--get", "core.logAllRefUpdates"])
    run(["--git-dir", bare, "config", "--show-origin", "--get", "gc.reflogExpire"])
    run(["--git-dir", bare, "cat-file", "-e", f"{candidate}^{{commit}}"])
    ancestor = run(["--git-dir", bare, "merge-base", "--is-ancestor", candidate, "refs/heads/main"], expected=1)
    if current != base:
        raise AssertionError("the target ref was not moved back to the base commit")
    if candidate not in reflog:
        raise AssertionError("the accepted candidate is missing from the branch reflog")
    run(["--git-dir", bare, "fsck", "--full", "--unreachable", "--no-reflogs"])
    report = (
        f"git_version={version}\nbase_commit={base}\ncandidate_commit={candidate}\n"
        f"current_ref_commit={current}\ncandidate_is_current_ancestor=false\n"
        f"candidate_present_in_branch_reflog=true\ncandidate_object_still_readable=true\n"
        f"forgejo_global_reflog_settings=core.logAllRefUpdates=true,gc.reflogExpire=90 days\n"
        f"ancestor_probe_output={ancestor!r}\n"
    )
    (root / "result.txt").write_text(report)
    (root / "commands.txt").write_text("\n".join(commands) + "\n")
    (root / "tool-versions.txt").write_text(
        f"Git: {version}\nPython: {sys.version.split()[0]}\nOS: {os.uname().sysname} {os.uname().release} {os.uname().machine}\n"
        "Config isolation: GIT_CONFIG_GLOBAL points to scratch/forgejo-internal.gitconfig; GIT_CONFIG_NOSYSTEM=1; user HOME untouched\n"
    )
    (root / "final.log").write_text("".join(log))
    (root / "final.exit").write_text("0\n")
except Exception as exc:
    (root / "final.log").write_text("".join(log) + f"HARNESS_ERROR: {exc}\n")
    (root / "final.exit").write_text("1\n")
    raise
