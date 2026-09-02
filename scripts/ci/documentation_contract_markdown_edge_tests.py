import tempfile
import unittest
from pathlib import Path

import check_documentation_contracts


ENGLISH_CURSOR_LIMIT = """Cursor's v1 installer registers MCP only. The verified `observe` and
`summarize` runtime commands exist, but `remem install --target cursor` does
not install automatic capture hooks or SessionStart injection.
The current public report does not support public benchmark claims.
directional_only_no_public_claim
"""
CHINESE_CURSOR_LIMIT = """Cursor v1 安装器只注册 MCP。已经验证的 `observe` 和 `summarize` runtime
命令可以使用，但 `remem install --target cursor` 不会安装自动捕获 hook，也没有
SessionStart 注入。
当前 public report 不能用于对外 benchmark 声明。
directional_only_no_public_claim
"""


class MarkdownEdgeContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="remem-markdown-edge-")
        self.root = Path(self.temp_dir.name)
        (self.root / "docs").mkdir()
        (self.root / "README.md").write_text(
            "# remem\n\n" + ENGLISH_CURSOR_LIMIT, encoding="utf-8"
        )
        (self.root / "README.zh-CN.md").write_text(
            "# remem\n\n" + CHINESE_CURSOR_LIMIT, encoding="utf-8"
        )
        (self.root / "docs/README.md").write_text(
            "# Documentation\n", encoding="utf-8"
        )

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def local_link_violations(self, addition: str) -> list[str]:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8") + addition, encoding="utf-8"
        )
        violations: list[str] = []
        check_documentation_contracts.check_local_markdown_links(
            self.root, violations
        )
        return violations

    def test_local_links_support_indented_atx_heading_anchors(self) -> None:
        (self.root / "docs/indented.md").write_text(
            "  ## Installation\n", encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Install](docs/indented.md#installation)\n"
        )
        self.assertFalse(any("docs/indented.md#installation" in item for item in violations))

    def test_local_links_validate_multiline_inline_destinations(self) -> None:
        violations = self.local_link_violations(
            "\n[Guide](\ndocs/missing-multiline.md)\n"
        )
        self.assertTrue(any("docs/missing-multiline.md" in item for item in violations))

    def test_local_links_require_a_real_opening_label(self) -> None:
        violations = self.local_link_violations(
            "\nLiteral Markdown syntax: ](docs/not-a-link.md)\n"
        )
        self.assertFalse(any("docs/not-a-link.md" in item for item in violations))

    def test_bilingual_invariants_require_affirmative_cursor_limitation(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "does\nnot install automatic capture hooks",
                "does\ninstall automatic capture hooks",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(any("Cursor v1 limitation" in item for item in violations))

    def test_local_links_ignore_html_comments(self) -> None:
        violations = self.local_link_violations(
            "\n<!-- [Draft](docs/missing-commented.md) -->\n"
        )
        self.assertFalse(any("docs/missing-commented.md" in item for item in violations))

    def test_local_links_honor_even_backslash_delimiters(self) -> None:
        violations = self.local_link_violations(
            "\n[label\\\\](docs/missing-after-backslash.md)\n"
        )
        self.assertTrue(any("docs/missing-after-backslash.md" in item for item in violations))

    def test_local_links_validate_rendered_html_attributes(self) -> None:
        violations = self.local_link_violations(
            '\n<a href="docs/missing-html.md">Guide</a>\n'
            '<img src="assets/missing-html.png" alt="missing">\n'
        )
        self.assertTrue(any("docs/missing-html.md" in item for item in violations))
        self.assertTrue(any("assets/missing-html.png" in item for item in violations))

    def test_local_links_validate_markdown_extension_fragments(self) -> None:
        (self.root / "docs/guide.markdown").write_text("# Existing\n", encoding="utf-8")
        violations = self.local_link_violations(
            "\n[Missing](docs/guide.markdown#missing)\n"
        )
        self.assertTrue(any("missing Markdown anchor" in item for item in violations))

    def test_bilingual_invariants_require_affirmative_public_claim_boundary(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "does not support public benchmark claims",
                "supports public benchmark claims",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(any("public benchmark claim boundary" in item for item in violations))

    def test_bilingual_invariants_ignore_html_comments(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "The current public report does not support public benchmark claims.",
                "<!-- The current public report does not support public benchmark claims. -->",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(any("public benchmark claim boundary" in item for item in violations))

    def test_local_links_support_explicit_html_anchors(self) -> None:
        (self.root / "docs/anchor.md").write_text(
            '<a name="configuration"></a>\n', encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Configuration](docs/anchor.md#configuration)\n"
        )
        self.assertFalse(any("docs/anchor.md" in item for item in violations))

    def test_local_links_ignore_non_link_html_attributes(self) -> None:
        violations = self.local_link_violations(
            '\n<script src="docs/not-rendered.js"></script>\n'
            '<div href="docs/not-a-link.md">Example</div>\n'
        )
        self.assertFalse(any("not-rendered.js" in item for item in violations))
        self.assertFalse(any("not-a-link.md" in item for item in violations))

    def test_local_links_preserve_combining_marks_in_heading_anchors(self) -> None:
        (self.root / "docs/combining.md").write_text(
            "## Cafe\u0301\n", encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Café](docs/combining.md#café)\n"
        )
        self.assertFalse(any("docs/combining.md" in item for item in violations))

    def test_local_links_exclude_front_matter_from_heading_anchors(self) -> None:
        (self.root / "docs/front-matter.md").write_text(
            "---\ntitle: Guide\n---\n\n# Real heading\n", encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Not a heading](docs/front-matter.md#title-guide)\n"
        )
        self.assertTrue(any("missing Markdown anchor" in item for item in violations))

    def test_bilingual_invariants_ignore_script_contents(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "The current public report does not support public benchmark claims.",
                "<script>The current public report does not support public benchmark claims.</script>",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(any("public benchmark claim boundary" in item for item in violations))

    def test_local_links_exclude_quoted_front_matter_keys(self) -> None:
        (self.root / "docs/quoted-front-matter.md").write_text(
            '---\n"title": Guide\n---\n\n# Real heading\n', encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Not a heading](docs/quoted-front-matter.md#title-guide)\n"
        )
        self.assertTrue(any("missing Markdown anchor" in item for item in violations))

    def test_bilingual_invariants_ignore_struck_through_prose(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "The current public report does not support public benchmark claims.",
                "~~The current public report does not support public benchmark claims.~~",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(any("public benchmark claim boundary" in item for item in violations))

    def test_local_links_ignore_anchors_inside_hidden_html(self) -> None:
        (self.root / "docs/hidden-anchor.md").write_text(
            '<template><a id="configuration"></a></template>\n', encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Configuration](docs/hidden-anchor.md#configuration)\n"
        )
        self.assertTrue(any("missing Markdown anchor" in item for item in violations))

    def test_bilingual_invariants_ignore_inline_hidden_html_text(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "The current public report does not support public benchmark claims.",
                "Visible <template>The current public report does not support public "
                "benchmark claims.</template>",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(any("public benchmark claim boundary" in item for item in violations))

    def test_heading_anchors_ignore_inline_hidden_html_text(self) -> None:
        anchors = check_documentation_contracts.heading_anchors(
            "# Visible <template>hidden</template> tail\n"
        )
        self.assertNotIn("visible-hidden-tail", anchors)
        self.assertIn("visible--tail", anchors)

    def test_bilingual_route_invariants_reject_external_suffix_match(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8")
            + "\n[Security](https://example.com/SECURITY.md)\n",
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(
            any(
                item.startswith("README.md:") and "security policy" in item
                for item in violations
            )
        )

    def test_bilingual_route_invariants_accept_local_destination(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8") + "\n[Security](SECURITY.md)\n",
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertFalse(
            any(
                item.startswith("README.md:") and "security policy" in item
                for item in violations
            )
        )

    def test_bilingual_invariants_ignore_cross_block_hidden_text(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "The current public report does not support public benchmark claims.",
                "<template>\n\nThe current public report does not support public "
                "benchmark claims.\n\n</template>",
            ),
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(
            any(
                item.startswith("README.md:")
                and "public benchmark claim boundary" in item
                for item in violations
            )
        )

    def test_bilingual_routes_ignore_cross_block_hidden_links(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8")
            + "\n<template>\n\n[Security](SECURITY.md)\n\n</template>\n",
            encoding="utf-8",
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        self.assertTrue(
            any(
                item.startswith("README.md:") and "security policy" in item
                for item in violations
            )
        )

    def test_heading_anchors_ignore_cross_block_hidden_heading(self) -> None:
        (self.root / "docs/hidden-heading.md").write_text(
            "<template>\n\n# Configuration\n\n</template>\n", encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Configuration](docs/hidden-heading.md#configuration)\n"
        )
        self.assertTrue(any("missing Markdown anchor" in item for item in violations))


if __name__ == "__main__":
    unittest.main()
