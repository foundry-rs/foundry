import pathlib
import re
import subprocess
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "tempo-check.sh"


class TempoAnvilReadinessTests(unittest.TestCase):
    def run_wait(self, setup, timeout="120"):
        # Load only the helper, without running the live-chain checks.
        helper = re.search(r"^wait_for_anvil\(\) \{.*?^\}", SCRIPT.read_text(), re.M | re.S)
        self.assertIsNotNone(helper)
        return subprocess.run(
            ["bash", "-c", helper.group() + r'''
set -euo pipefail
sleep 60 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
# Advance the readiness clock without waiting for real RPC timeouts.
sleep() { SECONDS=$((SECONDS + $1)); }
attempts=0
cast() {
  [[ "$*" == "client --rpc-url http://127.0.0.1:8547 --rpc-timeout 1" ]] || exit 99
  attempts=$((attempts + 1))
  (( attempts >= ready_after ))
}
eval "$1"
if wait_for_anvil "$pid" 8547 "$2"; then
  result=0
else
  result=$?
fi
echo "attempts=$attempts"
exit "$result"
''', "bash", setup, timeout],
            text=True,
            capture_output=True,
            timeout=5,
            check=False,
        )

    def test_fork_startup_beyond_old_deadline(self):
        result = self.run_wait("ready_after=13")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Anvil started successfully on port 8547", result.stdout)
        self.assertIn("attempts=13", result.stdout)

    def test_exited_child_fails_before_probing_rpc(self):
        result = self.run_wait('ready_after=1; kill "$pid"; wait "$pid" || true')
        self.assertEqual(result.returncode, 1)
        self.assertIn("Anvil exited before serving RPC", result.stderr)
        self.assertIn("attempts=0", result.stdout)

    def test_unresponsive_child_times_out(self):
        result = self.run_wait("ready_after=1000", timeout="3")
        self.assertEqual(result.returncode, 1)
        self.assertIn("within 3s", result.stderr)
        self.assertIn("attempts=3", result.stdout)


if __name__ == "__main__":
    unittest.main()
