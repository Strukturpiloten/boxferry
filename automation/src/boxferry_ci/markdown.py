"""Bounded opaque code projection for explicitly named maintainer prose files.

This is deliberately not a Markdown renderer. It recognizes unambiguous root
fences/indented blocks, single-line inline spans and complete code/pre HTML.
Unknown containers, escapes, malformed delimiters and ambiguous HTML select full
validation. Every retained region includes its original bytes and delimiters.
"""

from __future__ import annotations

import re
from bisect import bisect_right

MAX_MARKDOWN_BYTES = 1024 * 1024
MAX_REGIONS = 4096
MAX_LINES = 20000
MAX_LINE_BYTES = 65536
_FENCE = re.compile(rb"( {0,3})(`{3,}|~{3,})([^\r\n]*)(?:\r?\n)?\Z")
_TICKS = re.compile(rb"`+")
_LIST = re.compile(rb" {0,3}(?:[-+*]|[0-9]+[.)])[ \t]+")
_UNSUPPORTED_CONTAINER = re.compile(rb" {0,3}(?:>|\[[^\]]+\]:|:[ \t]+|:{3,}|!{3}|\?{3})")
_HEADING = re.compile(rb"#{1,6}(?:[ \t]+|\r?$)")
_HTML_OPEN = re.compile(
    rb"<(?P<tag>code|pre)(?:[ \t]+[A-Za-z_:][A-Za-z0-9_:.-]*"
    rb"(?:[ \t]*=[ \t]*(?:\"[^\"<>\r\n]*\"|'[^'<>\r\n]*'|[A-Za-z0-9_.:/-]+))?)*[ \t]*>",
    re.IGNORECASE,
)
_HTML_NESTED = re.compile(rb"<(?:/?[A-Za-z]|!|\?)")


def _html_end(data: bytes, start: int) -> int | None:
    opening = _HTML_OPEN.match(data, start)
    if opening is None:
        return None
    closing = re.compile(rb"</" + opening["tag"] + rb"[ \t]*>", re.IGNORECASE)
    end = closing.search(data, opening.end())
    if end is None or _HTML_NESTED.search(data, opening.end(), end.start()):
        return None
    return end.end()


def _inline(line: bytes) -> tuple[tuple[str, bytes], ...] | None:
    regions: list[tuple[str, bytes]] = []
    outside: list[bytes] = []
    position = 0
    while position < len(line):
        if len(regions) >= MAX_REGIONS:
            return None
        marker = re.search(rb"[`<]", line[position:])
        if marker is None:
            outside.append(line[position:])
            break
        start = position + marker.start()
        outside.append(line[position:start])
        if start and line[start - 1] == ord("\\"):
            return None  # Escaped delimiters need rendering semantics we do not infer.
        if line[start] == ord("<"):
            end = _html_end(line, start)
            if end is None or line[start : start + 4].lower() == b"<pre":
                return None
            regions.append(("html-inline", line[start:end]))
        else:
            opening = _TICKS.match(line, start)
            assert opening is not None
            end = None
            for closing in _TICKS.finditer(line, opening.end()):
                if len(closing[0]) == len(opening[0]):
                    end = closing.end()
                    break
            if end is None:
                return None
            regions.append(("inline", line[start:end]))
        position = end
    prose = b"".join(outside)
    # Multiline link/image syntax can reinterpret an otherwise unchanged span.
    # Ordinary complete, code-free Markdown links remain prose.
    if prose.count(b"[") != prose.count(b"]"):
        return None
    if b"](" in prose and prose.count(b"(") != prose.count(b")"):
        return None
    if regions and (b"[" in prose or b"]" in prose):
        return None
    return tuple(regions)


def _indented(line: bytes) -> bool:
    return (
        bool(line.strip())
        and len(line) - len(line.lstrip(b" \t")) > 0
        and (len(line.expandtabs(4)) - len(line.expandtabs(4).lstrip(b" ")) >= 4)
    )


def code_projection(data: bytes) -> tuple[tuple[str, bytes], ...] | None:
    """Return exact ordered code regions, or None when interpretation is uncertain."""
    if len(data) > MAX_MARKDOWN_BYTES or any(
        value < 32 and value not in {9, 10, 13} for value in data
    ):
        return None
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return None
    lines = data.splitlines(keepends=True)
    if len(lines) > MAX_LINES or any(len(line) > MAX_LINE_BYTES for line in lines):
        return None
    first = next((line.strip() for line in lines if line.strip()), b"")
    if first in {b"---", b"+++"}:
        return None  # Frontmatter can hide code-shaped metadata from the renderer.
    offsets = [0]
    for line in lines:
        offsets.append(offsets[-1] + len(line))
    regions: list[tuple[str, bytes]] = []
    index = 0
    list_active = False
    blank = True
    while index < len(lines):
        if len(regions) > MAX_REGIONS:
            return None
        line = lines[index]
        if not line.strip():
            blank = True
            index += 1
            continue
        if _UNSUPPORTED_CONTAINER.match(line):
            return None
        fence = _FENCE.fullmatch(line)
        if fence is not None:
            if fence[1] or (fence[2].startswith(b"`") and b"`" in fence[3]):
                return None
            delimiter = fence[2][:1]
            closing = re.compile(
                rb" {0,3}"
                + delimiter
                + rb"{"
                + str(len(fence[2])).encode()
                + rb",}[ \t]*(?:\r?\n)?\Z"
            )
            end = index + 1
            while end < len(lines) and closing.fullmatch(lines[end]) is None:
                end += 1
            if end == len(lines):
                return None
            regions.append(("fence", b"".join(lines[index : end + 1])))
            index, blank, list_active = end + 1, False, False
            continue
        if _indented(line):
            if list_active or not blank:
                return None
            end, last_code = index, index
            while end < len(lines) and (_indented(lines[end]) or not lines[end].strip()):
                if lines[end].strip():
                    last_code = end
                end += 1
            regions.append(("indented", b"".join(lines[index : last_code + 1])))
            index, blank = last_code + 1, False
            continue
        if line.lstrip(b" \t").lower().startswith((b"<code", b"<pre")):
            if line.startswith((b" ", b"\t")) or list_active or not blank:
                return None
            start = offsets[index]
            end = _html_end(data, start)
            if end is None:
                return None
            last = bisect_right(offsets, end - 1) - 1
            if data[end : offsets[last + 1]].strip():
                return None
            if last + 1 < len(lines) and lines[last + 1].strip():
                return None
            regions.append(("html-block", data[start:end]))
            index, blank, list_active = last + 1, False, False
            continue
        if _LIST.match(line):
            list_active = True
        elif _HEADING.match(line) or (blank and not line.startswith((b" ", b"\t"))):
            list_active = False
        inline = _inline(line)
        if inline is None:
            return None
        regions.extend(inline)
        index, blank = index + 1, False
    return tuple(regions) if len(regions) <= MAX_REGIONS else None


def unchanged_code(contents: tuple[bytes, ...]) -> bool:
    """Compare all observed revisions, including index and working-tree evidence."""
    if len(contents) < 2:
        return False
    projections = [code_projection(content) for content in contents]
    return projections[0] is not None and all(value == projections[0] for value in projections)
