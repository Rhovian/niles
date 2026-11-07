"""Offline coverage for the nightly change gate; also run by CI."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/nightly-changes.sh"


class NightlyChanges(unittest.TestCase):
    def test_release_history_and_failures(self):
        nightly = dict(tag_name="v1.13.1-nightly.1.1", published_at="2026-11-01",
                       prerelease=True, draft=False, target_commitish="abc")
        newer = dict(nightly, published_at="2026-11-02", target_commitish="def")
        excluded = [dict(nightly, prerelease=False), dict(nightly, draft=True),
                    dict(nightly, tag_name="v1.14.0-beta.1")]
        for pages, sha, gh_exit, expected in [
            ([[]], "abc", 0, "true"),
            ([excluded], "abc", 0, "true"),
            ([[nightly]], "abc", 0, "false"),
            ([[nightly]], "def", 0, "true"),
            ([[nightly, *excluded], [newer]], "def", 0, "false"),
            ([[]], "abc", 7, None),
            ("invalid json", "abc", 0, None),
            ([[dict(nightly, target_commitish=None)]], "abc", 0, None),
        ]:
            with self.subTest(pages=pages, sha=sha, gh_exit=gh_exit):
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    gh = root / "gh"
                    gh.write_text('#!/bin/sh\nprintf "%s" "$RELEASES"; exit "$GH_EXIT"\n')
                    gh.chmod(0o755)
                    env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}", RUNNER_TEMP=directory,
                               GITHUB_REPOSITORY="owner/repo", GITHUB_SHA=sha, GH_EXIT=str(gh_exit),
                               RELEASES=pages if isinstance(pages, str) else json.dumps(pages))
                    result = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True)
                    if expected is None:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertEqual(result.stdout, "")
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(result.stdout.strip(), expected)


if __name__ == "__main__":
    unittest.main()
