#!/usr/bin/env python3
"""Compiler-resolved Rust public-surface discovery for the lifecycle guard."""

from __future__ import annotations

import hashlib
import html
import re
import subprocess
from html.parser import HTMLParser
from pathlib import Path

from surface_lifecycle_evidence import rustdoc_signature

RUSTDOC_TOOLCHAIN = "1.97.0"


def _associated_name(anchor_name: str) -> str:
    return re.sub(r"-([0-9]+)$", r"#\1", anchor_name.replace(".", "::"))


def _header_signature(fragment: str, level: int) -> str:
    match = re.search(rf'<h{level} class="code-header">(.*?)</h{level}>', fragment, re.S)
    if not match:
        raise RuntimeError("rustdoc implementation signature is missing")
    signature = html.unescape(re.sub(r"<[^>]+>", "", match.group(1)))
    return hashlib.sha256(re.sub(r"\s+", " ", signature).strip().encode()).hexdigest()


def _local_source_location(root: Path, fragment: str) -> tuple[Path, int, int] | None:
    match = re.search(
        r'<a class="src rightside" href="[^"]*/src/remem/(?P<path>[^"#]+\.rs)\.html#'
        r'(?P<start>\d+)(?:-(?P<end>\d+))?"',
        fragment,
    )
    if not match:
        return None
    path = root / "src" / match.group("path")
    start = int(match.group("start"))
    return path, start, int(match.group("end") or start)


def _explicit_trait_implementations(root: Path, page_text: str) -> tuple[list[tuple[str, str]], list[tuple[Path, int, int]]]:
    section = page_text.split('id="trait-implementations-list"', 1)
    if len(section) != 2:
        return [], []
    body = section[1].split('id="synthetic-implementations"', 1)[0]
    implementations: list[tuple[str, str]] = []
    ranges: list[tuple[Path, int, int]] = []
    for match in re.finditer(
        r'<section id="(?P<id>impl-[^"]+)" class="impl">(?P<body>.*?)</section>',
        body,
        re.S,
    ):
        location = _local_source_location(root, match.group("body"))
        if location is None or not location[0].is_file():
            continue
        source = "\n".join(location[0].read_text(encoding="utf-8").splitlines()[location[1] - 1 : location[2]])
        if not re.search(r"\bimpl(?:\s*<[^>{}]*>)?\s+[^{}]+\s+for\s+[^{}]+", source, re.S):
            continue
        implementations.append((match.group("id"), _header_signature(match.group("body"), 3)))
        ranges.append(location)
    return implementations, ranges


class _AllItemsParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.in_all_items = False
        self.list_depth = 0
        self.href: str | None = None
        self.anchor_text: list[str] = []
        self.items: list[tuple[str, str]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if tag == "ul" and attributes.get("class") == "all-items":
            self.in_all_items = True
            self.list_depth = 1
            return
        if self.in_all_items and tag == "ul":
            self.list_depth += 1
        if self.in_all_items and tag == "a":
            self.href = attributes.get("href")
            self.anchor_text = []

    def handle_data(self, data: str) -> None:
        if self.href is not None:
            self.anchor_text.append(data)

    def handle_endtag(self, tag: str) -> None:
        if self.in_all_items and tag == "a" and self.href is not None:
            item = "".join(self.anchor_text).strip()
            if item and self.href:
                self.items.append((item, self.href))
            self.href = None
        if self.in_all_items and tag == "ul":
            self.list_depth -= 1
            if self.list_depth == 0:
                self.in_all_items = False


def discover_rust_exports(root: Path, *, doc_root: Path | None = None) -> set[str]:
    """Use rustdoc's compiler-resolved public graph and fingerprint declarations."""
    if doc_root is None:
        version = subprocess.run(
            ["rustc", "--version"], text=True, capture_output=True, check=False,
        )
        if version.returncode != 0 or not version.stdout.startswith(f"rustc {RUSTDOC_TOOLCHAIN} "):
            raise RuntimeError(
                f"surface discovery requires rustdoc {RUSTDOC_TOOLCHAIN}, got {version.stdout.strip() or version.stderr.strip()}"
            )
        result = subprocess.run(
            ["cargo", "doc", "--locked", "--quiet", "--no-deps", "--lib"],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise RuntimeError(f"cargo doc failed while discovering Rust exports: {detail}")
        doc_root = root / "target/doc/remem"

    all_items = doc_root / "all.html"
    if not all_items.is_file():
        raise RuntimeError(f"rustdoc public-item index is missing: {all_items}")
    parser = _AllItemsParser()
    parser.feed(all_items.read_text(encoding="utf-8"))
    if not parser.items:
        raise RuntimeError(f"rustdoc public-item index has no all-items list: {all_items}")

    exports: set[str] = set()
    for item, href in parser.items:
        page = (all_items.parent / href).resolve()
        try:
            page.relative_to(doc_root.resolve())
        except ValueError as exc:
            raise RuntimeError(f"rustdoc item link escapes crate docs: {href}") from exc
        if not page.is_file():
            raise RuntimeError(f"rustdoc linked public item page is missing: {page}")
        page_text = page.read_text(encoding="utf-8")
        exports.add(f"remem::{item}@sha256={rustdoc_signature(page_text)}")
        associated: dict[str, str] = {}
        explicit_impls, explicit_ranges = _explicit_trait_implementations(root, page_text)
        exports.update(
            f"remem::{item}::impl:{name}@sha256={digest}" for name, digest in explicit_impls
        )
        for category, name in re.findall(
            r'id="(structfield|variant|tymethod)\.([A-Za-z_][A-Za-z0-9_.-]*)"', page_text
        ):
            associated[_associated_name(name)] = f"{category}.{name}"
        implementations = re.search(
            r'id="implementations-list"(?P<body>.*?)(?:id="trait-implementations"|$)',
            page_text,
            re.S,
        )
        if implementations:
            for category, name, classes in re.findall(
                r'<section\s+id="(method|associatedconstant|associatedtype)\.([A-Za-z_][A-Za-z0-9_-]*)"\s+class="([^"]+)"',
                implementations.group("body"),
            ):
                if "trait-impl" not in classes.split():
                    associated[_associated_name(name)] = f"{category}.{name}"
        for match in re.finditer(
            r'<section\s+id="(?P<anchor>(?:method|associatedconstant|associatedtype)\.'
            r'(?P<name>[A-Za-z_][A-Za-z0-9_-]*))"\s+class="[^"]*\btrait-impl\b[^"]*"'
            r'(?P<body>.*?)</section>',
            page_text,
            re.S,
        ):
            location = _local_source_location(root, match.group("body"))
            if location and any(
                location[0] == path and start <= location[1] <= end
                for path, start, end in explicit_ranges
            ):
                associated[_associated_name(match.group("name"))] = match.group("anchor")
        if "/trait." in href or href.rsplit("/", 1)[-1].startswith("trait."):
            declarations = page_text.split('id="implementations"', 1)[0]
            for category, name in re.findall(
                r'id="(method|associatedconstant|associatedtype)\.([A-Za-z_][A-Za-z0-9_-]*)"',
                declarations,
            ):
                associated[_associated_name(name)] = f"{category}.{name}"
        exports.update(
            f"remem::{item}::{name}@sha256={rustdoc_signature(page_text, anchor)}"
            for name, anchor in associated.items()
        )
    queue = [doc_root / "index.html"]
    visited: set[Path] = set()
    module_link = re.compile(r'<a\s+class="[^"]*\bmod\b[^"]*"\s+href="([^"]+/index\.html)"')
    while queue:
        page = queue.pop()
        if page in visited:
            continue
        visited.add(page)
        if not page.is_file():
            raise RuntimeError(f"rustdoc linked public module page is missing: {page}")
        for href in module_link.findall(page.read_text(encoding="utf-8")):
            child = (page.parent / href).resolve()
            try:
                relative = child.relative_to(doc_root.resolve())
            except ValueError as exc:
                raise RuntimeError(f"rustdoc module link escapes crate docs: {href}") from exc
            exports.add("remem::" + "::".join(relative.parts[:-1]))
            queue.append(child)
    return exports
