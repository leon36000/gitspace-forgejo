from __future__ import annotations

from contextlib import redirect_stderr
import io
import json
import os
from pathlib import Path
import runpy
import signal
import subprocess
import sys
import tempfile
import time
from unittest import mock
import unittest


ROOT = Path(__file__).resolve().parents[1]
SUPERVISOR = ROOT / "scripts/mission_012_mcp_supervisor.py"
SUPERVISOR_MODULE = runpy.run_path(str(SUPERVISOR))
SUPERVISOR_GLOBALS = SUPERVISOR_MODULE["kill_verifier_scope"].__globals__
sys.path.insert(0, str(ROOT / "scripts"))
import run_mission_012_live_mcp_agent_e2e as agent  # noqa: E402
FAKE_HOST = (
    "import sys\n"
    "for frame in sys.stdin.buffer:\n"
    "    sys.stdout.buffer.write(frame)\n"
    "    sys.stdout.buffer.flush()\n"
    "sys.stdout.write('{\"event\":\"host_finalization\",\"outcome\":\"candidate_ready\"}\\n')\n"
    "sys.stdout.flush()\n"
)
SILENT_AFTER_EOF_HOST = (
    "import os, sys, time\n"
    "for _frame in sys.stdin.buffer:\n"
    "    pass\n"
    "print('HOSTPID ' + str(os.getpid()), flush=True)\n"
    "time.sleep(60)\n"
)
OVERSIZED_OUTPUT_HOST = (
    "import os\n"
    "chunk = b'x' * 65536\n"
    "for _ in range(32):\n"
    "    view = memoryview(chunk)\n"
    "    while view:\n"
    "        view = view[os.write(1, view):]\n"
)


class Mission012McpSupervisorTests(unittest.TestCase):
    def test_bounded_cli_refuses_to_launch_without_attested_agent_scope(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "cli-was-launched"
            command = (
                "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('launched')"
            )
            status, stdout, stderr, timed_out, exceeded = agent.run_bounded_cli(
                [sys.executable, "-c", command, str(marker)],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                timeout=5,
            )

            self.assertEqual(status, 125)
            self.assertEqual(stdout, b"")
            self.assertIn(b"attested systemd process scope", stderr)
            self.assertFalse(timed_out)
            self.assertFalse(exceeded)
            self.assertFalse(marker.exists(), "CLI launched outside the required cgroup scope")

    def test_cgroup_kill_access_failure_uses_only_validated_user_scope(self) -> None:
        uid = os.getuid()
        unit = f"run-fallback-test-{os.getpid()}.scope"
        scope_path = (
            f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/{unit}"
        )
        environment = {
            "PATH": "/usr/bin:/bin",
            "HOME": "/nonexistent",
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "XDG_RUNTIME_DIR": f"/run/user/{uid}",
            "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{uid}/bus",
        }
        with (
            mock.patch.object(
                SUPERVISOR_MODULE["os"],
                "open",
                side_effect=PermissionError("injected cgroup permission refusal"),
            ),
            mock.patch.dict(
                SUPERVISOR_GLOBALS,
                {"verifier_systemd_environment": lambda: environment},
            ),
            mock.patch.object(SUPERVISOR_MODULE["subprocess"], "run") as systemctl,
        ):
            self.assertTrue(SUPERVISOR_MODULE["kill_verifier_scope"](scope_path))

        systemctl.assert_called_once()
        command = systemctl.call_args.args[0]
        self.assertEqual(
            command,
            [
                "/usr/bin/systemctl",
                "--user",
                "kill",
                "--kill-whom=all",
                "--signal=SIGKILL",
                unit,
            ],
        )
        self.assertEqual(systemctl.call_args.kwargs["env"], environment)
        self.assertEqual(systemctl.call_args.kwargs["timeout"], 2)
        self.assertEqual(
            SUPERVISOR_MODULE["verifier_scope_unit"](scope_path), unit
        )
        self.assertIsNone(
            SUPERVISOR_MODULE["verifier_scope_unit"](
                f"/user.slice/user-{uid}.slice/user@{uid}.service/system.slice/{unit}"
            )
        )
        self.assertIsNone(
            SUPERVISOR_MODULE["verifier_scope_unit"](
                f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/run-é.scope"
            )
        )

    def test_cgroup_kill_and_user_bus_failure_is_reported_as_incomplete(self) -> None:
        uid = os.getuid()
        scope_path = (
            f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/"
            f"run-unavailable-fallback-{os.getpid()}.scope"
        )
        diagnostic = io.StringIO()
        with (
            mock.patch.object(
                SUPERVISOR_MODULE["os"],
                "open",
                side_effect=PermissionError("injected cgroup permission refusal"),
            ),
            mock.patch.dict(
                SUPERVISOR_GLOBALS,
                {"verifier_systemd_environment": lambda: None},
            ),
            mock.patch.object(SUPERVISOR_MODULE["subprocess"], "run") as systemctl,
            redirect_stderr(diagnostic),
        ):
            self.assertFalse(SUPERVISOR_MODULE["kill_verifier_scope"](scope_path))

        systemctl.assert_not_called()
        self.assertIn("validated systemd user bus unavailable", diagnostic.getvalue())

    def test_terminate_host_group_reports_rejected_verifier_kill_request(self) -> None:
        class FakeHost:
            pid = 42

            @staticmethod
            def wait(timeout: float) -> int:
                return 0

            @staticmethod
            def poll() -> int:
                return 0

        uid = os.getuid()
        scope_paths = [
            f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/run-{index}.scope"
            for index in range(2)
        ]
        kill_scope = mock.Mock(side_effect=[True, False])

        def killpg(_pid: int, requested_signal: int) -> None:
            if requested_signal == 0:
                raise ProcessLookupError
            if requested_signal == signal.SIGKILL:
                raise ProcessLookupError

        with (
            mock.patch.dict(
                SUPERVISOR_GLOBALS,
                {
                    "verifier_scope_paths": lambda _pid: scope_paths,
                    "kill_verifier_scope": kill_scope,
                },
            ),
            mock.patch.object(
                SUPERVISOR_MODULE["os"], "killpg", side_effect=killpg
            ) as mocked_killpg,
        ):
            cleanup_requested = SUPERVISOR_MODULE["terminate_host_group"](FakeHost())

        self.assertFalse(cleanup_requested)
        self.assertEqual(
            kill_scope.call_args_list,
            [mock.call(scope_path) for scope_path in scope_paths],
        )
        self.assertEqual(mocked_killpg.call_args_list[-1], mock.call(42, signal.SIGKILL))

    def test_foreign_scope_path_is_refused_before_cgroup_or_systemd_access(self) -> None:
        uid = os.getuid()
        foreign_scope = (
            f"/user.slice/user-{uid}.slice/user@{uid}.service/system.slice/"
            f"run-foreign-scope-{os.getpid()}.scope"
        )
        diagnostic = io.StringIO()
        with (
            mock.patch.object(SUPERVISOR_MODULE["os"], "open") as cgroup_open,
            mock.patch.object(SUPERVISOR_MODULE["subprocess"], "run") as systemctl,
            redirect_stderr(diagnostic),
        ):
            self.assertFalse(SUPERVISOR_MODULE["kill_verifier_scope"](foreign_scope))

        cgroup_open.assert_not_called()
        systemctl.assert_not_called()
        self.assertIn("refusing invalid verifier scope path", diagnostic.getvalue())

    def test_claude_output_capture_enforces_one_combined_byte_budget(self) -> None:
        code = "import os; os.write(1,b'a'*64); os.write(2,b'b'*64)"
        status, stdout, stderr, timed_out, exceeded = agent._run_bounded_cli_process_group(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            timeout=5,
            byte_limit=80,
        )

        self.assertFalse(timed_out)
        self.assertTrue(exceeded)
        self.assertNotEqual(status, 0)
        self.assertLessEqual(len(stdout) + len(stderr), 80)

    def test_claude_timeout_after_pipe_eof_kills_descendants(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            marker = Path(temp) / "child-ran-after-timeout"
            child = "import pathlib,sys,time; time.sleep(0.6); pathlib.Path(sys.argv[1]).write_text('escaped')"
            parent = (
                "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[2]],"
                "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); "
                "sys.stdout.close(); sys.stderr.close(); time.sleep(60)"
            )

            status, stdout, stderr, timed_out, exceeded = agent._run_bounded_cli_process_group(
                [sys.executable, "-c", parent, child, str(marker)],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                timeout=0.15,
                byte_limit=1024,
            )
            time.sleep(0.7)

            self.assertTrue(timed_out)
            self.assertFalse(exceeded)
            self.assertEqual((stdout, stderr), (b"", b""))
            self.assertNotEqual(status, 0)
            self.assertFalse(marker.exists())

    def test_parent_sigterm_cleans_up_separate_claude_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            marker = Path(temp) / "child-ran-after-parent-cancel"
            pid_file = Path(temp) / "child-pid"
            child = (
                "import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text(str(__import__('os').getpid())); "
                "time.sleep(0.6); pathlib.Path(sys.argv[2]).write_text('escaped')"
            )
            wrapper = (
                "import pathlib,sys; sys.path.insert(0,sys.argv[1]); "
                "import run_mission_012_live_mcp_agent_e2e as m; "
                "r=m._run_bounded_cli_process_group([sys.executable,'-c',sys.argv[2],sys.argv[3],sys.argv[4]], "
                "cwd=pathlib.Path.cwd(), env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'}, timeout=10); "
                "raise SystemExit(1 if r[3] else 0)"
            )
            process = subprocess.Popen(
                [sys.executable, "-c", wrapper, str(ROOT / "scripts"), child, str(pid_file), str(marker)],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            deadline = time.monotonic() + 5
            while not pid_file.exists() and time.monotonic() < deadline and process.poll() is None:
                time.sleep(0.02)
            self.assertTrue(pid_file.exists(), "bounded CLI child did not start")
            os.kill(process.pid, signal.SIGTERM)
            status = process.wait(timeout=5)
            time.sleep(0.7)

            self.assertNotEqual(status, 0)
            self.assertFalse(marker.exists())

    def test_host_runs_outside_supervisor_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            group_file = Path(temp) / "host-pgid"
            host = "import os,sys; open(sys.argv[1], 'w').write(f'{os.getpid()} {os.getpgrp()}')"
            process = subprocess.Popen(
                [sys.executable, str(SUPERVISOR), "--", sys.executable, "-c", host, str(group_file)],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            expected_pgid = os.getpgid(process.pid)
            status = process.wait(timeout=10)

            self.assertEqual(status, 0)
            host_pid, host_pgid = map(int, group_file.read_text().split())
            self.assertEqual(host_pid, host_pgid)
            self.assertNotEqual(host_pgid, expected_pgid)

    def test_parent_cancellation_allows_mcp_host_to_finalize(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            ready_file = Path(temp) / "host-ready"
            final_file = Path(temp) / "host-finalized"
            host = (
                "import os,pathlib,sys; pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); "
                "sys.stdin.buffer.read(); pathlib.Path(sys.argv[2]).write_text('finalized'); "
                "print('{\"event\":\"host_finalization\",\"outcome\":\"candidate_ready\"}',flush=True)"
            )
            claude = (
                "import os,pathlib,subprocess,sys,time; pathlib.Path(sys.argv[5]).write_text(str(os.getpid())); "
                "subprocess.Popen([sys.executable,sys.argv[1],'--',sys.executable,'-c',"
                "sys.argv[2],sys.argv[3],sys.argv[4]],stdin=subprocess.PIPE,stdout=subprocess.PIPE); "
                "time.sleep(60)"
            )
            wrapper = (
                "import pathlib,signal,sys; sys.path.insert(0,sys.argv[1]); "
                "signal.pthread_sigmask(signal.SIG_BLOCK,{signal.SIGTERM}); "
                "import run_mission_012_live_mcp_agent_e2e as module; "
                "result=module._run_bounded_cli_process_group([sys.executable,'-c',sys.argv[2],sys.argv[3],"
                "sys.argv[4],sys.argv[5],sys.argv[6],sys.argv[7]],cwd=pathlib.Path.cwd(), "
                "env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'},timeout=30); "
                "raise SystemExit(0 if result[3] else 1)"
            )
            claude_pid_file = Path(temp) / "claude-pid"
            process = subprocess.Popen(
                [sys.executable, "-c", wrapper, str(ROOT / "scripts"), claude,
                 str(SUPERVISOR), host, str(ready_file), str(final_file), str(claude_pid_file)],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            deadline = time.monotonic() + 5
            while not ready_file.exists() and time.monotonic() < deadline and process.poll() is None:
                time.sleep(0.02)
            self.assertTrue(ready_file.exists(), "MCP host did not start")
            os.kill(process.pid, signal.SIGTERM)
            try:
                status = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
                if claude_pid_file.exists():
                    try:
                        os.killpg(int(claude_pid_file.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if ready_file.exists():
                    try:
                        os.killpg(int(ready_file.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                self.fail("bounded CLI cancellation did not finish")
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                if claude_pid_file.exists():
                    try:
                        os.killpg(int(claude_pid_file.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if ready_file.exists():
                    try:
                        os.killpg(int(ready_file.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass

            self.assertEqual(status, 0)
            self.assertEqual(final_file.read_text(), "finalized")

    def test_sigterm_arriving_as_cleanup_starts_does_not_skip_group_termination(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            claude_pid_file = Path(temp) / "claude-pid"
            ready_file = Path(temp) / "descendant-ready"
            marker = Path(temp) / "descendant-survived"
            child = (
                "import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text('ready'); "
                "time.sleep(0.35); pathlib.Path(sys.argv[2]).write_text('escaped')"
            )
            claude = (
                "import os,pathlib,subprocess,sys,time; pathlib.Path(sys.argv[2]).write_text(str(os.getpid())); "
                "subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[3],sys.argv[4]],"
                "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); "
                "deadline=time.monotonic()+3\n"
                "while not pathlib.Path(sys.argv[3]).exists() and time.monotonic()<deadline: time.sleep(0.01)\n"
                "if not pathlib.Path(sys.argv[3]).exists(): raise SystemExit(2)\n"
                "os.write(1,b'completed\\n')"
            )
            wrapper = (
                "import os,pathlib,signal,sys; sys.path.insert(0,sys.argv[1]); "
                "import run_mission_012_live_mcp_agent_e2e as module; "
                "original=module._terminate_cli_group\n"
                "def inject(process):\n"
                " os.kill(os.getpid(), signal.SIGTERM)\n"
                " return original(process)\n"
                "module._terminate_cli_group=inject\n"
                "result=module._run_bounded_cli_process_group([sys.executable,'-c',sys.argv[2],sys.argv[3],"
                "sys.argv[4],sys.argv[5],sys.argv[6]],cwd=pathlib.Path.cwd(), "
                "env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'},timeout=5)\n"
                "raise SystemExit(0 if result[3] and result[0] == 128 + signal.SIGTERM else 1)"
            )
            process = subprocess.Popen(
                [sys.executable, "-c", wrapper, str(ROOT / "scripts"), claude, child,
                 str(claude_pid_file), str(ready_file), str(marker)],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            try:
                status = process.wait(timeout=5)
                time.sleep(0.5)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                if process.poll() is None:
                    process.wait(timeout=5)

            self.assertEqual(status, 0)
            self.assertTrue(ready_file.exists(), "descendant did not start before CLI completion")
            self.assertFalse(marker.exists())

    def test_sigterm_after_eof_bounds_wait_for_silent_host(self) -> None:
        process = subprocess.Popen(
            [sys.executable, str(SUPERVISOR), "--", sys.executable, "-c", SILENT_AFTER_EOF_HOST],
            cwd=ROOT,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        host_pid: int | None = None
        try:
            process.stdin.close()
            pid_line = process.stdout.readline()
            self.assertTrue(pid_line.startswith(b"HOSTPID "), pid_line)
            host_pid = int(pid_line.split()[1])
            os.kill(process.pid, signal.SIGTERM)
            status = process.wait(timeout=17)
            self.assertEqual(status, 75)
            self.assertEqual(process.stdout.read(), b"")
        finally:
            if host_pid is not None:
                try:
                    os.killpg(host_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if process.poll() is None:
                process.kill()
                process.wait(timeout=3)
            process.stdout.close()
            process.stdin.close()

    def test_supervisor_uses_distinct_status_when_verifier_cleanup_is_unconfirmed(self) -> None:
        uid = os.getuid()
        scope_path = (
            f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/"
            f"run-unconfirmed-cleanup-{os.getpid()}.scope"
        )
        with tempfile.TemporaryDirectory(prefix="mcp-supervisor-cleanup-status-") as directory:
            root = Path(directory)
            pid_path = root / "host.pid"
            launcher = root / "inject_cleanup_failure.py"
            child_source = (
                f"import os,time; open({str(pid_path)!r}, 'w').write(str(os.getpid())); "
                "time.sleep(60)"
            )
            launcher.write_text(
                "import runpy, sys\n"
                f"module = runpy.run_path({str(SUPERVISOR)!r})\n"
                "globals_ = module['main'].__globals__\n"
                "globals_['HOST_FINALIZATION_TIMEOUT_SECONDS'] = 0.15\n"
                f"globals_['verifier_scope_paths'] = lambda _pid: [{scope_path!r}]\n"
                "globals_['kill_verifier_scope'] = lambda _path: False\n"
                f"sys.argv = [{str(SUPERVISOR)!r}, '--', '/usr/bin/python3', '-c', "
                f"{child_source!r}]\n"
                "raise SystemExit(module['main']())\n",
                encoding="utf-8",
            )

            process = subprocess.Popen(
                [sys.executable, str(launcher)],
                cwd=ROOT,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            )
            assert process.stdin is not None
            process.stdin.close()
            process.stdin = None
            time.sleep(0.05)
            process.send_signal(signal.SIGTERM)
            try:
                stdout, stderr = process.communicate(timeout=5)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=3)

            self.assertEqual(process.returncode, 76, stderr.decode(errors="replace"))
            self.assertTrue(pid_path.exists(), "the bounded host did not start")
            host_pid = int(pid_path.read_text(encoding="ascii"))
            with self.assertRaises(ProcessLookupError):
                os.kill(host_pid, 0)

    def test_oversized_host_output_is_truncated_before_excess_bytes(self) -> None:
        process = subprocess.Popen(
            [sys.executable, str(SUPERVISOR), "--", sys.executable, "-c", OVERSIZED_OUTPUT_HOST],
            cwd=ROOT,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        assert process.stdout is not None
        try:
            output = process.stdout.read()
            status = process.wait(timeout=10)
            self.assertEqual(status, 65)
            self.assertEqual(output, b"x" * (128 * 1024))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=3)
            process.stdout.close()

    def test_eof_closes_host_input_and_forwards_finalization(self) -> None:
        process = subprocess.Popen(
            [sys.executable, str(SUPERVISOR), "--", sys.executable, "-c", FAKE_HOST],
            cwd=ROOT,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        process.stdin.write(b'{"jsonrpc":"2.0","id":1,"method":"ping"}\n')
        process.stdin.close()
        output = process.stdout.read()
        status = process.wait(timeout=10)
        process.stdout.close()
        process.stdin.close()
        self.assertEqual(status, 0)
        self.assertIn(b'"method":"ping"', output)
        self.assertIn(b'"event":"host_finalization"', output)

    def test_shutdown_barrier_keeps_the_actual_mcp_host_alive_until_gate_release(self) -> None:
        host = (
            "import json, os, pathlib, sys\n"
            "pathlib.Path(sys.argv[1]).write_text(str(os.getpid()))\n"
            "log = pathlib.Path(sys.argv[2])\n"
            "for line in sys.stdin:\n"
            " frame = json.loads(line)\n"
            " method = frame.get('method')\n"
            " with log.open('a') as stream: stream.write(method + '\\n')\n"
            " print(json.dumps({'jsonrpc':'2.0','id':frame.get('id'),'result':{}}), flush=True)\n"
            " if method == 'shutdown': break\n"
        )
        with tempfile.TemporaryDirectory(prefix="mcp-supervisor-gate-barrier-") as directory:
            root = Path(directory)
            root.chmod(0o700)
            runtime = root / ".runtime"
            runtime.mkdir(mode=0o700)
            ready = runtime / "mcp-gate-ready.json"
            release = runtime / "mcp-gate-release.json"
            host_pid_file = root / "host.pid"
            host_calls = root / "host-calls.log"
            process = subprocess.Popen(
                [
                    sys.executable,
                    str(SUPERVISOR),
                    "--gate-barrier-ready",
                    str(ready),
                    "--gate-barrier-release",
                    str(release),
                    "--",
                    sys.executable,
                    "-c",
                    host,
                    str(host_pid_file),
                    str(host_calls),
                ],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
            assert process.stdin is not None
            assert process.stdout is not None
            assert process.stderr is not None
            binding: dict[str, object] | None = None
            remaining_stdout = b""
            stderr_output = b""
            try:
                process.stdin.write(b'{"jsonrpc":"2.0","id":1,"method":"ping"}\n')
                process.stdin.flush()
                self.assertIn(b'"id": 1', process.stdout.readline())
                process.stdin.write(b'{"jsonrpc":"2.0","id":2,"method":"shutdown"}\n')
                process.stdin.flush()

                deadline = time.monotonic() + 3
                while not ready.exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(ready.is_file(), "the trusted supervisor did not publish the shutdown barrier")
                binding = json.loads(ready.read_bytes())
                host_pid = binding["host_pid"]
                self.assertEqual(binding["host_executable"], os.path.realpath(sys.executable))
                self.assertEqual(binding["supervisor_pid"], process.pid)
                self.assertTrue(host_pid_file.is_file())
                self.assertEqual(int(host_pid_file.read_text()), host_pid)
                os.kill(host_pid, 0)
                self.assertIsNone(process.poll(), "the MCP host supervisor exited before release")
                self.assertEqual(host_calls.read_text(encoding="utf-8").splitlines(), ["ping"])
            finally:
                nonce = binding.get("nonce") if isinstance(binding, dict) else "invalid"
                release.write_text(
                    json.dumps({
                        "schema_version": 1,
                        "nonce": nonce,
                        "gate_operation_sha256": "a" * 64,
                    }),
                    encoding="ascii",
                )
                release.chmod(0o600)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
                remaining_stdout = process.stdout.read()
                stderr_output = process.stderr.read()
                process.stdin.close()
                process.stdout.close()
                process.stderr.close()

            self.assertEqual(process.returncode, 0, stderr_output.decode(errors="replace"))
            self.assertIn(b'"id": 2', remaining_stdout)
            self.assertEqual(host_calls.read_text(encoding="utf-8").splitlines(), ["ping", "shutdown"])
            self.assertNotEqual(process.poll(), None)

    def test_sigterm_closes_host_input_and_allows_final_receipt(self) -> None:
        process = subprocess.Popen(
            [sys.executable, str(SUPERVISOR), "--", sys.executable, "-c", FAKE_HOST],
            cwd=ROOT,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        process.stdin.write(b'{"jsonrpc":"2.0","id":1,"method":"ping"}\n')
        process.stdin.flush()
        self.assertIn(b'"method":"ping"', process.stdout.readline())
        os.kill(process.pid, signal.SIGTERM)
        output = process.stdout.read()
        status = process.wait(timeout=10)
        process.stdout.close()
        process.stdin.close()
        self.assertEqual(status, 0)
        self.assertIn(b'"event":"host_finalization"', output)


if __name__ == "__main__":
    unittest.main()
