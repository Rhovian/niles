#!/usr/bin/env python3
"""Record argv and selected launch environment before executing a CI fixture CLI."""
import json
import os
from pathlib import Path
import sys


def main():
    argv = sys.argv[1:]
    if argv[:1] == ["--"]:
        argv = argv[1:]
    values = {key: os.environ[key] for key in json.loads(os.environ["NILES_CONFORMANCE_ENV_KEYS"])}
    record = Path(os.environ["NILES_CONFORMANCE_RECORD"])
    temporary = record.with_suffix(".tmp")
    # Publish a complete record. https://docs.python.org/3/library/os.html#os.replace
    temporary.write_text(json.dumps({"argv": argv, "env": values}, indent=2), encoding="utf-8")
    temporary.replace(record)
    cli = os.environ["NILES_CONFORMANCE_CLI"]
    os.execv(cli, [cli, *argv])


if __name__ == "__main__":
    main()
