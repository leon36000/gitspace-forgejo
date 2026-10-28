from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import run_mission_012_live_composed_e2e as composed  # noqa: E402


class Mission012LiveComposedTests(unittest.TestCase):
    def test_composed_runner_refuses_real_provider_before_reading_inputs(self) -> None:
        with (
            mock.patch.object(
                composed.os,
                "readlink",
                side_effect=AssertionError("provider refusal must precede namespace inspection"),
            ),
            mock.patch.object(
                composed,
                "capture_sources",
                side_effect=AssertionError("provider refusal must precede source capture"),
            ),
        ):
            with self.assertRaisesRegex(
                RuntimeError,
                "provider egress and filesystem isolation are not attested",
            ):
                composed.run(type("Args", (), {"agent_adapter": "claude"})())

    def test_parent_scope_stage_runner_refuses_without_attested_outer_scope(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-parent-scope-refusal-") as temporary:
            marker = Path(temporary) / "command-started"
            with mock.patch.object(
                composed,
                "current_attested_process_scope_snapshot",
                return_value=None,
            ):
                exit_code, output = composed.run_stage_command_in_attested_parent_scope(
                    [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).touch()", str(marker)],
                    2,
                )

        self.assertEqual(exit_code, 125)
        self.assertIn(b"attested parent cgroup scope is unavailable", output)
        self.assertFalse(marker.exists(), "an unscoped command must not start")

    def test_parent_scope_stage_runner_refuses_changed_cgroup_members(self) -> None:
        snapshot = composed.ProcessScopeSnapshot(
            "run-mission012-stage-1-0123456789abcdef.scope",
            "/user.slice/user-1000.slice/user@1000.service/app.slice/run-mission012-stage-1-0123456789abcdef.scope",
            (os.getpid(),),
        )
        with (
            mock.patch.object(
                composed,
                "current_attested_process_scope_snapshot",
                return_value=snapshot,
            ),
            mock.patch.object(
                composed,
                "process_scope_snapshot_matches",
                side_effect=(True, False),
            ),
        ):
            exit_code, output = composed.run_stage_command_in_attested_parent_scope(
                [sys.executable, "-c", "print('stage finished')"],
                5,
            )

        self.assertEqual(exit_code, 125)
        self.assertIn(b"parent_membership_changed", output)

    def test_parent_scope_stage_runner_detects_and_outer_scope_cleans_escaped_session(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-parent-scope-escaped-") as temporary:
            runtime = Path(temporary)
            child_pid_file = runtime / "escaped-child.pid"
            script_dir = str(ROOT / "scripts")
            child_code = "import time; time.sleep(60)"
            escaped_parent = (
                "import pathlib,subprocess,sys; "
                "child=subprocess.Popen([sys.executable,'-c',sys.argv[2]],"
                "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,"
                "stderr=subprocess.DEVNULL,close_fds=True,start_new_session=True); "
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid)); "
                "print('parent returned')"
            )
            inner = (
                "import sys; sys.path.insert(0,sys.argv[1]); "
                "import run_mission_012_live_composed_e2e as c; "
                "status,out=c.run_stage_command_in_attested_parent_scope("
                "[sys.executable,'-c',sys.argv[3],sys.argv[2],sys.argv[4]],5); "
                "print('inner_exit='+str(status)); print(out.decode('utf-8','replace'))"
            )
            exit_code, output = composed.run_stage_command(
                [
                    sys.executable, "-c", inner,
                    script_dir, str(child_pid_file), escaped_parent, child_code,
                ],
                15,
            )
            self.assertTrue(child_pid_file.exists(), output.decode("utf-8", "replace"))
            child_pid = int(child_pid_file.read_text())
            try:
                self.assertEqual(exit_code, 125, output.decode("utf-8", "replace"))
                self.assertIn(b"inner_exit=125", output)
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    try:
                        state = Path(f"/proc/{child_pid}/stat").read_bytes().rsplit(b")", 1)[1].split()[0]
                    except FileNotFoundError:
                        state = None
                        break
                    if state in {b"Z", b"X"}:
                        break
                    time.sleep(0.02)
                self.assertIn(state, {None, b"Z", b"X"}, "outer systemd scope left an escaped child alive")
            finally:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_stage_builder_refuses_real_provider_before_other_validation(self) -> None:
        with self.assertRaisesRegex(
            RuntimeError,
            "provider egress and filesystem isolation are not attested",
        ):
            composed.build_stage_commands(
                Path("/unavailable/repo"),
                Path("/tmp/unavailable-state"),
                Path("/unavailable/forgejo"),
                Path("/unavailable/evidence"),
                "bad-execution-id",
                "bad-allocation-id",
                "0",
                agent_adapter="claude",
            )

    def test_gate_barrier_is_only_added_to_sandboxed_local_adapter_agent_stage(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-gate-builder-") as temporary:
            repo = Path(temporary)
            (repo / "evidence").mkdir()
            state = Path("/tmp") / f"mission012-gate-builder-{Path(temporary).name}"
            evidence = repo / "evidence" / "mission-012-live-agent-webhook-gate-121"
            stages = composed.build_stage_commands(
                repo,
                state,
                Path("/tmp/forgejo-fixture"),
                evidence,
                "live-mcp-121",
                "mission012-gate-test",
                "0.20",
                agent_adapter="local-test-adapter",
                sandbox_test_adapter=True,
                same_host_gate_barrier=True,
                reuse_attested_parent_scope=True,
            )

        self.assertEqual([stage[0] for stage in stages], ["admission", "agent", "pull-request"])
        agent = stages[1][1]
        self.assertIn("--gate-barrier-ready-file", agent)
        self.assertIn("--gate-barrier-release-file", agent)
        self.assertIn("--reuse-attested-parent-scope", agent)
        ready = Path(agent[agent.index("--gate-barrier-ready-file") + 1])
        release = Path(agent[agent.index("--gate-barrier-release-file") + 1])
        self.assertEqual(ready.parent, release.parent)
        self.assertEqual(ready.parent, state / "project" / ".runtime")
        self.assertEqual(ready.name, "mcp-gate-ready.json")
        self.assertEqual(release.name, "mcp-gate-release.json")
        for _, command, _ in (stages[0], stages[2]):
            self.assertNotIn("--gate-barrier-ready-file", command)
            self.assertNotIn("--gate-barrier-release-file", command)

    def test_gate_barrier_refuses_model_and_unsandboxed_agents(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-gate-refusal-") as temporary:
            repo = Path(temporary)
            (repo / "evidence").mkdir()
            state = Path("/tmp") / f"mission012-gate-refusal-{Path(temporary).name}"
            evidence = repo / "evidence" / "mission-012-live-agent-webhook-gate-122"
            common = (
                repo, state, Path("/tmp/forgejo-fixture"), evidence,
                "live-mcp-122", "mission012-gate-test", "0.20",
            )
            with self.assertRaisesRegex(
                RuntimeError,
                "provider egress and filesystem isolation are not attested",
            ):
                composed.build_stage_commands(
                    *common,
                    agent_adapter="claude",
                    same_host_gate_barrier=True,
                )
            with self.assertRaisesRegex(RuntimeError, "sandboxed local-test-adapter"):
                composed.build_stage_commands(
                    *common,
                    agent_adapter="local-test-adapter",
                    same_host_gate_barrier=True,
                )
            with self.assertRaisesRegex(RuntimeError, "parent-scope reuse requires"):
                composed.build_stage_commands(
                    *common,
                    agent_adapter="local-test-adapter",
                    reuse_attested_parent_scope=True,
                )

    def test_stage_runner_releases_a_ready_gate_callback_once(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-gate-runner-") as temporary:
            runtime = Path(temporary)
            ready = runtime / "ready.json"
            release = runtime / "release.json"
            source = (
                "import json, pathlib, sys, time\n"
                "ready = pathlib.Path(sys.argv[1]); release = pathlib.Path(sys.argv[2])\n"
                "ready.write_text(json.dumps({'nonce':'fixture'}))\n"
                "deadline = time.monotonic() + 5\n"
                "while time.monotonic() < deadline and not release.exists(): time.sleep(.01)\n"
                "if not release.exists(): raise SystemExit(9)\n"
                "print('gate released')\n"
            )
            calls: list[dict[str, object]] = []

            def release_gate(path: Path) -> None:
                calls.append(json.loads(path.read_text()))
                release.write_text("released")

            exit_code, output = composed.run_stage_command(
                [sys.executable, "-c", source, str(ready), str(release)],
                10,
                gate_barrier_ready_file=ready,
                on_gate_barrier_ready=release_gate,
            )

        self.assertEqual(exit_code, 0)
        self.assertEqual(calls, [{"nonce": "fixture"}])
        self.assertIn(b"gate released", output)

    def test_stage_runner_terminates_waiting_child_when_gate_verifier_fails(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-gate-failure-") as temporary:
            runtime = Path(temporary)
            ready = runtime / "ready.json"
            release = runtime / "release.json"
            pid_file = runtime / "stage.pid"
            source = (
                "import os, pathlib, sys, time\n"
                "ready = pathlib.Path(sys.argv[1]); release = pathlib.Path(sys.argv[2])\n"
                "pathlib.Path(sys.argv[3]).write_text(str(os.getpid()))\n"
                "ready.write_text('{\\\"nonce\\\":\\\"fixture\\\"}')\n"
                "deadline = time.monotonic() + 10\n"
                "while time.monotonic() < deadline and not release.exists(): time.sleep(.01)\n"
                "raise SystemExit(0 if release.exists() else 9)\n"
            )

            def reject_gate(_: Path) -> None:
                raise RuntimeError("independent verifier refused the candidate")

            exit_code, _ = composed.run_stage_command(
                [sys.executable, "-c", source, str(ready), str(release), str(pid_file)],
                5,
                gate_barrier_ready_file=ready,
                on_gate_barrier_ready=reject_gate,
            )

            self.assertEqual(exit_code, 125)
            self.assertFalse(release.exists(), "a failed verifier must not release the waiting child")
            child_pid = int(pid_file.read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(child_pid, 0)

    def test_stage_runner_rejects_success_when_child_leaves_a_live_group_descendant(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-group-survivor-") as temporary:
            runtime = Path(temporary)
            child_pid_file = runtime / "background-child.pid"
            child = (
                "import time\n"
                "while True: time.sleep(1)\n"
            )
            parent = (
                "import pathlib, subprocess, sys\n"
                "child = subprocess.Popen([sys.executable, '-c', sys.argv[2]], "
                "stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, "
                "stderr=subprocess.DEVNULL, close_fds=True)\n"
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid))\n"
                "print('agent stage returned')\n"
            )

            exit_code, output = composed.run_stage_command(
                [sys.executable, "-c", parent, str(child_pid_file), child],
                5,
            )
            child_pid = int(child_pid_file.read_text())
            try:
                deadline = time.monotonic() + 2
                running = True
                while time.monotonic() < deadline:
                    try:
                        stat_line = Path(f"/proc/{child_pid}/stat").read_text()
                    except FileNotFoundError:
                        running = False
                        break
                    state = stat_line.rsplit(")", 1)[1].split()[0]
                    running = state not in {"Z", "X"}
                    if not running:
                        break
                    time.sleep(0.02)

                self.assertEqual(exit_code, 125, output.decode(errors="replace"))
                self.assertFalse(running, "a successful stage left an active child in its process group")
            finally:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_stage_runner_rejects_success_when_descendant_escapes_with_setsid(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-session-survivor-") as temporary:
            runtime = Path(temporary)
            child_pid_file = runtime / "detached-child.pid"
            child = "import time\nwhile True: time.sleep(1)\n"
            parent = (
                "import pathlib,subprocess,sys\n"
                "child=subprocess.Popen(['/usr/bin/setsid',sys.executable,'-c',sys.argv[2]],"
                "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,"
                "stderr=subprocess.DEVNULL,close_fds=True)\n"
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid))\n"
                "print('detached child started')\n"
            )

            exit_code, output = composed.run_stage_command(
                [sys.executable, "-c", parent, str(child_pid_file), child],
                5,
            )
            child_pid = int(child_pid_file.read_text())
            try:
                deadline = time.monotonic() + 2
                running = True
                while time.monotonic() < deadline:
                    try:
                        state = Path(f"/proc/{child_pid}/stat").read_bytes().rsplit(b")", 1)[1].split()[0]
                    except FileNotFoundError:
                        running = False
                        break
                    running = state not in {b"Z", b"X"}
                    if not running:
                        break
                    time.sleep(0.02)

                self.assertEqual(exit_code, 125, output.decode(errors="replace"))
                self.assertFalse(running, "a detached session survived the agent stage")
            finally:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_agent_cli_uses_nested_scope_and_refuses_a_detached_survivor(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-cli-scope-") as temporary:
            runtime = Path(temporary)
            pid_file = runtime / "detached-cli-child.json"
            parent_scope_file = runtime / "parent-scope.txt"
            child = (
                "import json,os,pathlib,sys,time; "
                "pathlib.Path(sys.argv[1]).write_text(json.dumps({'pid':os.getpid(),"
                "'cgroup':pathlib.Path('/proc/self/cgroup').read_text().strip()})); "
                "time.sleep(60)"
            )
            cli_parent = (
                "import pathlib,subprocess,sys,time\n"
                "subprocess.Popen(['/usr/bin/setsid',sys.executable,'-c',sys.argv[1],sys.argv[2]],"
                "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,close_fds=True)\n"
                "deadline=time.monotonic()+3\n"
                "while not pathlib.Path(sys.argv[2]).exists() and time.monotonic()<deadline: time.sleep(.01)\n"
                "if not pathlib.Path(sys.argv[2]).exists(): raise SystemExit(33)\n"
                "print('CLI parent completed')\n"
            )
            stage = (
                "import json,pathlib,sys; sys.path.insert(0,sys.argv[1]); "
                "import run_mission_012_live_mcp_agent_e2e as cli; "
                "pathlib.Path(sys.argv[3]).write_text(pathlib.Path('/proc/self/cgroup').read_text().strip()); "
                "clean=cli.run_bounded_cli([sys.executable,'-c','print(\"nested scope ready\")'],"
                "cwd=pathlib.Path.cwd(),env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'},timeout=5); "
                "escaped=cli.run_bounded_cli([sys.executable,'-c',sys.argv[4],sys.argv[5],sys.argv[6]],"
                "cwd=pathlib.Path.cwd(),env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'},timeout=5); "
                "print(json.dumps({'clean_status':clean[0],'clean_stdout':clean[1].decode(),"
                "'escaped_status':escaped[0],'escaped_stdout':escaped[1].decode(),"
                "'escaped_stderr':escaped[2].decode()})); "
                "raise SystemExit(0 if clean[0]==0 and escaped[0]==125 else 32)"
            )

            exit_code, output = composed.run_stage_command(
                [
                    sys.executable,
                    "-c",
                    stage,
                    str(Path(__file__).resolve().parents[1] / "scripts"),
                    str(pid_file),
                    str(parent_scope_file),
                    cli_parent,
                    child,
                    str(pid_file),
                ],
                15,
            )

            self.assertEqual(exit_code, 0, output.decode(errors="replace"))
            summary = json.loads(output.splitlines()[-1])
            self.assertEqual(summary["clean_status"], 0)
            self.assertIn("nested scope ready", summary["clean_stdout"])
            self.assertEqual(summary["escaped_status"], 125)
            self.assertIn("CLI parent completed", summary["escaped_stdout"])
            self.assertNotIn("could not be attested", summary["escaped_stderr"])
            detached = json.loads(pid_file.read_text())
            child_pid = detached["pid"]
            parent_cgroup = parent_scope_file.read_text().strip()
            self.assertNotEqual(detached["cgroup"], parent_cgroup)
            self.assertIn("run-mission012-stage-", detached["cgroup"])
            try:
                deadline = time.monotonic() + 2
                state = None
                while time.monotonic() < deadline:
                    try:
                        state = Path(f"/proc/{child_pid}/stat").read_bytes().rsplit(b")", 1)[1].split()[0]
                    except FileNotFoundError:
                        state = None
                        break
                    if state in {b"Z", b"X"}:
                        break
                    time.sleep(0.02)
                self.assertIn(state, {None, b"Z", b"X"}, "nested cgroup left a detached CLI child active")
            finally:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_parent_cleanup_removes_cli_scope_after_stage_is_killed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-cli-scope-cancel-") as temporary:
            runtime = Path(temporary)
            child_file = runtime / "nested-child.json"
            stage_pid_file = runtime / "stage.pid"
            child = (
                "import json,os,pathlib,sys,time; "
                "pathlib.Path(sys.argv[1]).write_text(json.dumps({'pid':os.getpid(),"
                "'cgroup':pathlib.Path('/proc/self/cgroup').read_text().strip()})); "
                "time.sleep(60)"
            )
            stage = (
                "import os,pathlib,sys; sys.path.insert(0,sys.argv[1]); "
                "pathlib.Path(sys.argv[3]).write_text(str(os.getpid())); "
                "import run_mission_012_live_mcp_agent_e2e as cli; "
                "cli.run_bounded_cli([sys.executable,'-c',sys.argv[4],sys.argv[2]],"
                "cwd=pathlib.Path.cwd(),env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'},timeout=60)"
            )
            result: list[tuple[int, bytes]] = []
            stage_thread = threading.Thread(
                target=lambda: result.append(
                    composed.run_stage_command(
                        [
                            sys.executable,
                            "-c",
                            stage,
                            str(Path(__file__).resolve().parents[1] / "scripts"),
                            str(child_file),
                            str(stage_pid_file),
                            child,
                        ],
                        20,
                    )
                ),
                daemon=True,
            )
            stage_thread.start()
            child_pid: int | None = None
            stage_pid: int | None = None
            try:
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline and (not stage_pid_file.exists() or not child_file.exists()):
                    time.sleep(0.02)
                self.assertTrue(stage_pid_file.exists(), "agent stage did not start")
                self.assertTrue(child_file.exists(), "nested CLI child did not start")
                stage_pid = int(stage_pid_file.read_text())
                child_pid = int(json.loads(child_file.read_text())["pid"])
                os.kill(stage_pid, signal.SIGKILL)
                stage_thread.join(timeout=10)
                self.assertFalse(stage_thread.is_alive(), "host did not complete cleanup after stage SIGKILL")
                self.assertEqual(len(result), 1)
                self.assertNotEqual(result[0][0], 0, result[0][1].decode(errors="replace"))
                deadline = time.monotonic() + 2
                state = None
                while time.monotonic() < deadline:
                    try:
                        state = Path(f"/proc/{child_pid}/stat").read_bytes().rsplit(b")", 1)[1].split()[0]
                    except FileNotFoundError:
                        state = None
                        break
                    if state in {b"Z", b"X"}:
                        break
                    time.sleep(0.02)
                self.assertIn(state, {None, b"Z", b"X"}, "interrupted stage left its nested CLI scope running")
            finally:
                if stage_thread.is_alive() and stage_pid is not None:
                    try:
                        os.kill(stage_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    stage_thread.join(timeout=10)
                if child_pid is None and child_file.exists():
                    child_pid = int(json.loads(child_file.read_text())["pid"])
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_attested_parent_scope_cancellation_cleans_then_fresh_scope_restarts_stage(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-parent-scope-restart-") as temporary:
            runtime = Path(temporary)
            first_harness_pid = runtime / "cancel-harness.pid"
            stage_pid_file = runtime / "cancel-stage.pid"
            first_scope_unit_file = runtime / "cancel-scope-unit.txt"
            cancelled_file = runtime / "cancelled.json"
            recovery_harness_pid = runtime / "recovery-harness.pid"
            recovery_scope_unit_file = runtime / "recovery-scope-unit.txt"
            scripts = str(ROOT / "scripts")
            cancelled_harness = f'''\
import pathlib, sys
sys.path.insert(0, {scripts!r})
import run_mission_012_live_composed_e2e as composed
pathlib.Path({str(first_harness_pid)!r}).write_text(str(__import__("os").getpid()))
snapshot = composed.current_attested_process_scope_snapshot()
if snapshot is None: raise SystemExit(83)
pathlib.Path({str(first_scope_unit_file)!r}).write_text(snapshot.unit)
try:
    status, output = composed.run_stage_command_in_attested_parent_scope(
        [sys.executable, "-c", {("import os,pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(60)")!r}, {str(stage_pid_file)!r}],
        60,
    )
    raise SystemExit(81 if status == 0 else 82)
except KeyboardInterrupt:
    pathlib.Path({str(cancelled_file)!r}).write_text("stage-cancelled-and-cleaned")
'''
            environment = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8"}
            first_scope = composed.prepare_process_scope(
                [sys.executable, "-c", cancelled_harness], environment, 20
            )
            first = composed.launch_scoped_process(
                first_scope,
                [sys.executable, "-c", cancelled_harness],
                environment,
                cwd=ROOT,
                timeout=15,
                merge_stderr=True,
            )
            self.assertIsNotNone(first.process)
            self.assertIsNotNone(first.cgroup_path)
            first_process = first.process
            assert first_process is not None and first.cgroup_path is not None
            first_output = bytearray()
            reader = threading.Thread(
                target=lambda: first_output.extend(first_process.stdout.read())
                if first_process.stdout is not None else None,
                daemon=True,
            )
            reader.start()
            try:
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline and not stage_pid_file.exists():
                    time.sleep(0.02)
                self.assertTrue(stage_pid_file.exists(), bytes(first_output).decode("utf-8", "replace"))
                self.assertTrue(first_harness_pid.exists())
                self.assertTrue(first_scope_unit_file.exists())
                stage_pid = int(stage_pid_file.read_text())
                harness_pid = int(first_harness_pid.read_text())
                os.kill(harness_pid, signal.SIGINT)
                first_process.wait(timeout=8)
                reader.join(timeout=3)
                self.assertFalse(reader.is_alive(), "cancelled scope output was not drained")
                self.assertEqual(first_process.returncode, 0, bytes(first_output).decode("utf-8", "replace"))
                self.assertTrue(cancelled_file.exists(), "KeyboardInterrupt was not handled after stage cleanup")
                self.assertEqual(
                    composed.finish_process_scope(first_scope, first.cgroup_path),
                    "clean",
                )
                deadline = time.monotonic() + 2
                state = None
                while time.monotonic() < deadline:
                    try:
                        state = Path(f"/proc/{stage_pid}/stat").read_bytes().rsplit(b")", 1)[1].split()[0]
                    except FileNotFoundError:
                        state = None
                        break
                    if state in {b"Z", b"X"}:
                        break
                    time.sleep(0.02)
                self.assertIn(state, {None, b"Z", b"X"}, "cancelled stage process remained active")
            finally:
                if first_process.poll() is None:
                    composed.finish_process_scope(first_scope, first.cgroup_path)
                reader.join(timeout=3)
                if first_process.stdout is not None:
                    first_process.stdout.close()

            recovery_harness = f'''\
import pathlib, sys
sys.path.insert(0, {scripts!r})
import run_mission_012_live_composed_e2e as composed
pathlib.Path({str(recovery_harness_pid)!r}).write_text(str(__import__("os").getpid()))
snapshot = composed.current_attested_process_scope_snapshot()
if snapshot is None: raise SystemExit(83)
pathlib.Path({str(recovery_scope_unit_file)!r}).write_text(snapshot.unit)
status, output = composed.run_stage_command_in_attested_parent_scope(
    [sys.executable, "-c", "print('fresh-scope-recovery')"], 5
)
print("recovery_exit=" + str(status))
sys.stdout.buffer.write(output)
raise SystemExit(status)
'''
            recovery_scope = composed.prepare_process_scope(
                [sys.executable, "-c", recovery_harness], environment, 15
            )
            recovery = composed.launch_scoped_process(
                recovery_scope,
                [sys.executable, "-c", recovery_harness],
                environment,
                cwd=ROOT,
                timeout=10,
                merge_stderr=True,
            )
            self.assertIsNotNone(recovery.process)
            self.assertIsNotNone(recovery.cgroup_path)
            recovery_process = recovery.process
            assert recovery_process is not None and recovery.cgroup_path is not None
            assert recovery_process.stdout is not None
            recovery_output = recovery_process.stdout.read()
            recovery_process.stdout.close()
            recovery_process.wait(timeout=10)
            self.assertEqual(recovery_process.returncode, 0, recovery_output.decode("utf-8", "replace"))
            self.assertIn(b"recovery_exit=0", recovery_output)
            self.assertIn(b"fresh-scope-recovery", recovery_output)
            self.assertTrue(recovery_scope_unit_file.exists())
            self.assertNotEqual(first_scope_unit_file.read_text(), recovery_scope_unit_file.read_text())
            self.assertEqual(
                composed.finish_process_scope(recovery_scope, recovery.cgroup_path),
                "clean",
            )

    def test_nested_scope_refuses_to_execute_after_its_parent_scope_stops(self) -> None:
        environment = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"}
        parent = composed.prepare_process_scope(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            environment,
            10,
        )
        parent_start = composed.launch_scoped_process(
            parent,
            [sys.executable, "-c", "import time; time.sleep(60)"],
            environment,
            cwd=ROOT,
            timeout=10,
            merge_stderr=False,
        )
        self.assertIsNotNone(parent_start.process)
        self.assertIsNotNone(parent_start.cgroup_path)
        marker = Path(tempfile.gettempdir()) / f"mission012-stale-parent-{os.getpid()}-{time.monotonic_ns()}"
        nested_start = None
        try:
            parent_cleanup = composed.finish_process_scope(parent, parent_start.cgroup_path)
            self.assertIn(parent_cleanup, {"descendants_terminated", "clean"})
            assert parent_start.process is not None
            parent_start.process.wait(timeout=5)
            stale_child_command = [
                sys.executable,
                "-c",
                "import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text('ran'); time.sleep(60)",
                str(marker),
            ]
            stale_child = composed.prepare_process_scope(
                stale_child_command,
                environment,
                5,
                owner_unit=parent.unit,
            )
            nested_start = composed.launch_scoped_process(
                stale_child,
                stale_child_command,
                environment,
                cwd=ROOT,
                timeout=5,
                merge_stderr=False,
            )
            self.assertIsNone(nested_start.process, nested_start.startup_stderr.decode(errors="replace"))
            self.assertIsNone(nested_start.cgroup_path)
            self.assertFalse(marker.exists(), "stale parent scope allowed a CLI command to run")
        finally:
            if parent_start.cgroup_path is not None:
                composed.finish_process_scope(parent, parent_start.cgroup_path)
            for stream in (
                parent_start.process.stdin if parent_start.process is not None else None,
                parent_start.process.stdout if parent_start.process is not None else None,
                parent_start.process.stderr if parent_start.process is not None else None,
            ):
                if stream is not None:
                    stream.close()
            if nested_start is not None and nested_start.cgroup_path is not None:
                composed.finish_process_scope(stale_child, nested_start.cgroup_path)
            if nested_start is not None and nested_start.process is not None:
                nested_start.process.wait(timeout=5)
                for stream in (nested_start.process.stdin, nested_start.process.stdout, nested_start.process.stderr):
                    if stream is not None:
                        stream.close()
            try:
                marker.unlink()
            except FileNotFoundError:
                pass

    def test_systemd_runtime_limit_cleans_agent_tree_after_supervisor_sigkill(self) -> None:
        with tempfile.TemporaryDirectory(prefix="composed-host-crash-") as temporary:
            runtime = Path(temporary)
            child_file = runtime / "orphan-candidate.json"
            scripts = str(Path(__file__).resolve().parents[1] / "scripts")
            child = (
                "import json,os,pathlib,sys,time; "
                "pathlib.Path(sys.argv[1]).write_text(json.dumps({'pid':os.getpid(),"
                "'cgroup':pathlib.Path('/proc/self/cgroup').read_text().strip()})); "
                "time.sleep(60)"
            )
            stage = (
                "import pathlib,sys; sys.path.insert(0,sys.argv[1]); "
                "import run_mission_012_live_mcp_agent_e2e as cli; "
                "cli.run_bounded_cli([sys.executable,'-c',sys.argv[3],sys.argv[2]],"
                "cwd=pathlib.Path.cwd(),env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'},timeout=60)"
            )
            host = (
                "import pathlib,sys; sys.path.insert(0,sys.argv[1]); "
                "import run_mission_012_live_composed_e2e as composed; "
                "composed.run_stage_command([sys.executable,'-c',sys.argv[2],sys.argv[1],sys.argv[3],sys.argv[4]],2)"
            )
            supervisor = subprocess.Popen(
                [sys.executable, "-c", host, scripts, stage, str(child_file), child],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            child_pid: int | None = None
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline and not child_file.exists():
                    if supervisor.poll() is not None:
                        break
                    time.sleep(0.02)
                self.assertTrue(child_file.exists(), "scoped agent tree did not start")
                child_record = json.loads(child_file.read_text())
                child_pid = int(child_record["pid"])
                self.assertIn("run-mission012-stage-", child_record["cgroup"])
                os.kill(supervisor.pid, signal.SIGKILL)
                supervisor.wait(timeout=5)

                deadline = time.monotonic() + 10
                state = None
                while time.monotonic() < deadline:
                    try:
                        state = Path(f"/proc/{child_pid}/stat").read_bytes().rsplit(b")", 1)[1].split()[0]
                    except FileNotFoundError:
                        state = None
                        break
                    if state in {b"Z", b"X"}:
                        break
                    time.sleep(0.05)
                self.assertIn(
                    state,
                    {None, b"Z", b"X"},
                    "systemd RuntimeMaxSec failed to stop the stage and its bound CLI scope",
                )
                active = subprocess.run(
                    ["systemctl", "--user", "list-units", "--state=active", "--type=scope", "--no-legend", "run-mission012-stage-*"],
                    env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "XDG_RUNTIME_DIR": f"/run/user/{os.getuid()}", "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{os.getuid()}/bus"},
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    timeout=5,
                    check=False,
                )
                self.assertEqual(active.returncode, 0, active.stderr.decode(errors="replace"))
                self.assertEqual(active.stdout.strip(), b"")
            finally:
                if supervisor.poll() is None:
                    supervisor.kill()
                    supervisor.wait(timeout=5)
                if child_pid is not None:
                    try:
                        os.kill(child_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass


class TestAdapterIssueObservationTests(unittest.TestCase):
    def test_composed_gate_binds_adapter_observation_to_admitted_context_presence(self) -> None:
        composed.validate_test_adapter_issue_observation(
            {"untrusted_work_item_admitted": True},
            {"test_adapter_untrusted_work_item_observed": True},
            agent_adapter="local-test-adapter",
        )
        composed.validate_test_adapter_issue_observation(
            {"untrusted_work_item_admitted": False},
            {"test_adapter_untrusted_work_item_observed": False},
            agent_adapter="local-test-adapter",
        )

        for admission, agent in (
            ({"untrusted_work_item_admitted": True}, {}),
            (
                {"untrusted_work_item_admitted": True},
                {"test_adapter_untrusted_work_item_observed": False},
            ),
            (
                {"untrusted_work_item_admitted": False},
                {"test_adapter_untrusted_work_item_observed": True},
            ),
            (
                {"untrusted_work_item_admitted": 1},
                {"test_adapter_untrusted_work_item_observed": True},
            ),
        ):
            with self.subTest(admission=admission, agent=agent):
                with self.assertRaisesRegex(RuntimeError, "does not match admitted issue context"):
                    composed.validate_test_adapter_issue_observation(
                        admission,
                        agent,
                        agent_adapter="local-test-adapter",
                    )

    def test_model_run_cannot_claim_the_fixture_observed_issue_context(self) -> None:
        composed.validate_test_adapter_issue_observation(
            {"untrusted_work_item_admitted": True},
            {"test_adapter_untrusted_work_item_observed": None},
            agent_adapter="claude",
        )
        with self.assertRaisesRegex(RuntimeError, "model run cannot claim a test-adapter observation"):
            composed.validate_test_adapter_issue_observation(
                {"untrusted_work_item_admitted": True},
                {"test_adapter_untrusted_work_item_observed": True},
                agent_adapter="claude",
            )


if __name__ == "__main__":
    unittest.main()
