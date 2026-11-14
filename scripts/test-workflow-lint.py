#!/usr/bin/env python3
"""Exercise the selected installed Actionlint CLI with authored inert workflows.

Fixtures are authored MPL-2.0 and redistributable. They execute no Actions or
workflow commands and use no network. The inventory remains the version authority;
accepting a future publisher's metadata does not qualify or install its artifacts.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "automation/src"))

from boxferry_ci import runner, tooling


class ContractError(ValueError):
    """A bounded, closed-vocabulary failure without raw child diagnostics."""


def version(binary: Path, expected: str, directory: Path, environment: dict[str, str]) -> None:
    result = runner.run(
        (str(binary), "-version"),
        cwd=directory,
        stage="Actionlint version identity",
        timeout_seconds=5,
        max_output_bytes=8192,
        environment=environment,
    )
    lines = result.stdout.splitlines()
    if (
        result.status != "success"
        or not lines
        or lines[0] not in {expected, expected.removeprefix("v")}
    ):
        raise ContractError("installed Actionlint differs from the canonical inventory version")


def workflow(label: str) -> str:
    return (
        "---\nname: Authored workflow lint fixture\non: push\njobs:\n"
        f"  check:\n    runs-on: {label}\n    steps:\n      - run: echo checked\n"
    )


def exercise(binary: Path, manifest: Path = tooling.MANIFEST) -> int:
    inventory = tooling.load_manifest(manifest)
    selected = inventory["actionlint", tooling.host_architecture()]
    # An explicit allowlist in the canonical reader admits publishers; this selects
    # the expected current runner contract without inferring support from versions.
    label = "ubuntu-26.04" if selected.repository == "kjanat/actionlint" else "ubuntu-24.04"
    binary = binary.absolute()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ContractError("Actionlint executable is missing or not executable")
    with tempfile.TemporaryDirectory(prefix="boxferry-workflow-lint-") as temporary:
        directory = Path(temporary)
        environment = {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(directory),
            "XDG_CONFIG_HOME": str(directory),
            "NO_COLOR": "1",
        }
        version(binary, selected.tag, directory, environment)
        try:
            version(binary, "v0.0.0", directory, environment)
        except ContractError:
            pass
        else:
            raise ContractError("Actionlint version mismatch was accepted")
        cases = (
            ("known-runner", workflow(label), None),
            ("known-arm-runner", workflow(label + "-arm"), None),
            ("unknown-runner", workflow("ubuntu-999.04"), "[runner-label]"),
            ("conflicting-os", workflow(f"[{label}, windows-2022]"), "[runner-label]"),
            (
                "invalid-expression",
                workflow(label).replace("echo checked", "echo ${{ nonexistent.value }}"),
                "[expression]",
            ),
            ("malformed-yaml", "---\nname: malformed\non: [push\n", "[syntax-check]"),
        )
        for name, content, diagnostic in cases:
            path = directory / (name + ".yml")
            path.write_text(content, encoding="utf-8")
            result = runner.run(
                (
                    str(binary),
                    "-no-color",
                    "-format",
                    "{{range .}}{{.Message}} [{{.Kind}}]{{end}}",
                    str(path),
                ),
                cwd=directory,
                stage="Actionlint " + name,
                timeout_seconds=5,
                max_output_bytes=16384,
                environment=environment,
            )
            output = result.stdout + result.stderr
            if diagnostic is None:
                passed = result.status == "success" and result.returncode == 0 and not output
            else:
                passed = (
                    result.status == "failed" and result.returncode == 1 and diagnostic in output
                )
            if not passed:
                expected = "success" if diagnostic is None else "exit 1 with " + diagnostic
                raise ContractError(
                    f"{name}: expected {expected}; "
                    f"observed {result.status}, exit {result.returncode}"
                )
            print(f"Actionlint {name}: expected actual CLI result observed")
    print(
        f"Actionlint {selected.repository} {selected.tag}: "
        "version and six workflow contracts passed"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    executable = shutil.which("actionlint")
    if executable is None:
        raise ContractError("install canonical Actionlint before running its contracts")
    return exercise(Path(executable))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ContractError as error:
        print(f"Actionlint CLI regression contract failed: {error}", file=sys.stderr)
        raise SystemExit(1) from None
    except (OSError, ValueError):
        print(
            "Actionlint CLI regression contracts failed; check selected inventory and executable.",
            file=sys.stderr,
        )
        raise SystemExit(1) from None
