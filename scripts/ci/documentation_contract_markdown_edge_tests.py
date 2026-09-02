import tempfile
import unittest
from pathlib import Path

import check_documentation_contracts


ENGLISH_CURSOR_LIMIT = """Cursor's v1 installer registers MCP only. The verified `observe` and
`summarize` runtime commands exist, but `remem install --target cursor` does
not install automatic capture hooks or SessionStart injection.
"""
CHINESE_CURSOR_LIMIT = """Cursor v1 安装器只注册 MCP。已经验证的 `observe` 和 `summarize` runtime
命令可以使用，但 `remem install --target cursor` 不会安装自动捕获 hook，也没有
SessionStart 注入。
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


if __name__ == "__main__":
    unittest.main()
