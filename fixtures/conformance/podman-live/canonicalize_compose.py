#!/usr/bin/env python3
"""Canonicalize BoxFerry-generated Compose YAML for semantic comparisons."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


SORTED_RESOURCE_SECTIONS = {"services:", "networks:", "volumes:", "configs:", "secrets:"}
EMPTY_MAPPING_SUFFIX = ": {}\n"


def entry_key(block: list[str]) -> str:
    return block[0][2:].split(":", maxsplit=1)[0]


def normalize_entry(section: str, block: list[str]) -> list[str]:
    if section not in {"networks:", "volumes:"} or "    external: true\n" not in block:
        return block
    redundant_name = f"    name: {entry_key(block)}\n"
    return [line for line in block if line != redundant_name]


def canonicalize_section(section: str, lines: list[str]) -> list[str]:
    lines = [
        line.removesuffix(EMPTY_MAPPING_SUFFIX) + ":\n"
        if line.startswith("  ")
        and not line.startswith("    ")
        and line.endswith(EMPTY_MAPPING_SUFFIX)
        else line
        for line in lines
    ]
    blocks: list[list[str]] = []
    current: list[str] = []
    for line in lines:
        if line.startswith("  ") and not line.startswith("   ") and line.rstrip().endswith(":"):
            if current:
                blocks.append(normalize_entry(section, current))
            current = [line]
        elif current:
            current.append(line)
        elif line.strip():
            raise ValueError(f"unexpected content before first {section} entry")
    if current:
        blocks.append(normalize_entry(section, current))
    blocks.sort(key=entry_key)
    return [line for block in blocks for line in block]


def canonicalize(lines: list[str]) -> list[str]:
    output: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        output.append(line)
        index += 1
        section = line.rstrip("\n")
        if section not in SORTED_RESOURCE_SECTIONS:
            continue
        end = index
        while end < len(lines) and (lines[end].startswith(" ") or not lines[end].strip()):
            end += 1
        output.extend(canonicalize_section(section, lines[index:end]))
        index = end
    return output


def require_omission(report: dict, subject: str, reason: str) -> None:
    matching = [
        item for item in report.get("diagnostics", [])
        if item.get("code") == "BFQ0003"
        and item.get("severity") == "warning"
        and item.get("fields") == [
            {"name": "subject", "value": subject},
            {"name": "reason", "value": reason},
        ]
    ]
    if report.get("status") != "success" or len(matching) != 1:
        raise ValueError("missing exact reviewed Quadlet omission diagnostic")


def reviewed_quadlet_alias_loss(text: str, prefix: str, report: dict) -> str:
    """Account only for the authored dual-network API service's diagnosed loss."""
    service = f"{prefix}-large-api"
    require_omission(
        report, f"services.{service}.networks",
        "IP, IP6, and NetworkAlias require exactly one compatible network attachment",
    )
    lines = text.splitlines(keepends=True)
    header = f"  {service}:\n"
    if lines.count(header) != 1:
        raise ValueError("missing unique reviewed API service")
    start = lines.index(header)
    end = start + 1
    while end < len(lines) and lines[end].startswith("    "):
        end += 1
    block = "".join(lines[start:end])
    for network, alias in [("private", "api"), ("edge", "public-api")]:
        attachment = f"      {prefix}-large-{network}:"
        expected = f"{attachment}\n        aliases:\n          - {alias}\n"
        if block.count(expected) != 1:
            raise ValueError("missing exact reviewed per-network alias")
        block = block.replace(expected, f"{attachment} {{}}\n", 1)
    return "".join(lines[:start]) + block + "".join(lines[end:])


def reviewed_quadlet_restart_loss(text: str, prefix: str, report: dict) -> str:
    """Remove only the options fixture's independently authored finite retry policy."""
    lines = text.splitlines(keepends=True)
    header = f"  {prefix}-options:\n"
    if header not in lines:
        return text
    if lines.count(header) != 1:
        raise ValueError("duplicate reviewed options service")
    start = lines.index(header)
    end = start + 1
    while end < len(lines) and lines[end].startswith("    "):
        end += 1
    block = lines[start:end]
    restart = "    restart: on-failure:3\n"
    if block.count(restart) != 1:
        raise ValueError("missing exact reviewed options restart policy")
    require_omission(
        report, f"services.{prefix}-options.restart_policy",
        "a finite container restart count has no equivalent in Restart=; "
        "systemd start-rate limits use different time-window semantics",
    )
    block.remove(restart)
    return "".join(lines[:start] + block + lines[end:])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--reviewed-quadlet-losses", nargs=2, metavar=("PREFIX", "REPORT"))
    args = parser.parse_args()
    lines = args.input.read_text(encoding="utf-8").splitlines(keepends=True)
    if not lines or lines[0] != "---\n":
        raise SystemExit("expected a complete BoxFerry-generated YAML document")
    text = "".join(canonicalize(lines))
    if args.reviewed_quadlet_losses:
        prefix, report_path = args.reviewed_quadlet_losses
        report = json.loads(Path(report_path).read_text(encoding="utf-8"))
        text = reviewed_quadlet_alias_loss(text, prefix, report)
        text = reviewed_quadlet_restart_loss(text, prefix, report)
    args.output.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
