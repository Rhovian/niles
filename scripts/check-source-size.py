"""Keep handwritten source files small enough to review by responsibility."""

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
LIMIT = 500
EXTENSIONS = {".rs", ".py", ".sh", ".ts", ".tsx", ".js", ".css", ".astro"}
paths = [ROOT / "build.rs"]
for directory in ("src", "tests", "scripts", "apps"):
    paths.extend(path for path in (ROOT / directory).rglob("*")
                 if path.suffix in EXTENSIONS
                 and not {"node_modules", "dist", ".astro"}.intersection(path.parts))
failures = 0
for path in sorted(paths):
    lines = len(path.read_text().splitlines())
    if lines > LIMIT:
        print(f"{path.relative_to(ROOT)}: {lines} lines exceeds {LIMIT}", file=sys.stderr)
        failures += 1
if failures:
    sys.exit(1)
print(f"Source size: {len(paths)} files within {LIMIT} lines")
