#!/usr/bin/env python3
"""Canonicalize BoxFerry-generated Compose YAML for semantic comparisons."""

from __future__ import annotations

import argparse
from pathlib import Path


SORTED_RESOURCE_SECTIONS = {"services:", "networks:", "volumes:", "configs:", "secrets:"}


def entry_key(block: list[str]) -> str:
    return block[0][2:-2]


def normalize_entry(section: str, block: list[str]) -> list[str]:
    if section not in {"networks:", "volumes:"} or "    external: true\n" not in block:
        return block
    redundant_name = f"    name: {entry_key(block)}\n"
    return [line for line in block if line != redundant_name]


def canonicalize_section(section: str, lines: list[str]) -> list[str]:
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    lines = args.input.read_text(encoding="utf-8").splitlines(keepends=True)
    if not lines or lines[0] != "---\n":
        raise SystemExit("expected a complete BoxFerry-generated YAML document")
    args.output.write_text("".join(canonicalize(lines)), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
