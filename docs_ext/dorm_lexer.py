"""Pygments lexer for the DORM ``.schema`` DSL (the Seiva data-layer schema).

The DSL is Prisma-shaped: top-level ``datasource`` / ``generator`` blocks, a
``namespace`` per module, and ``model`` / ``enum`` blocks whose fields carry
``@attributes``.  This lexer exists so the documentation site can syntax-
highlight ```` ```schema ```` (and ``dorm``) fenced blocks the same way the
VS Code TextMate grammar (``editors/vscode/syntaxes/dorm-schema.tmLanguage.json``)
highlights ``*.schema`` files in the editor.

It is registered with Pygments by ``docs_ext/mkdocs_hooks.py`` (an mkdocs hook),
so no packaging / entry point is required for the docs build.
"""

from pygments.lexer import RegexLexer, bygroups, words
from pygments.token import (
    Comment,
    Keyword,
    Name,
    Number,
    Operator,
    Punctuation,
    String,
    Text,
    Whitespace,
)

__all__ = ["DormSchemaLexer"]

# Block-introducing keywords (`.k` in the Material/Pygments stylesheet).
_BLOCK_KEYWORDS = (
    "datasource",
    "generator",
    "namespace",
    "model",
    "enum",
    "type",
    "view",
    "service",
)

# Built-in scalar field types (`.kt`). Model/enum names (Capitalised) fall
# through to Name.Class (`.nc`).
_SCALAR_TYPES = (
    "String",
    "Int",
    "BigInt",
    "SmallInt",
    "Float",
    "Decimal",
    "Boolean",
    "Bool",
    "DateTime",
    "Date",
    "Time",
    "Uuid",
    "Json",
    "Jsonb",
    "Bytes",
    "Text",
)


class DormSchemaLexer(RegexLexer):
    """Lexer for DORM ``.schema`` files and ```` ```schema ```` fenced blocks."""

    name = "DORM schema"
    aliases = ["dorm", "dormschema", "dorm-schema", "schema"]
    filenames = ["*.schema"]
    mimetypes = ["text/x-dorm-schema"]
    url = "https://ngolacloud.github.io/seiva/"

    tokens = {
        "root": [
            (r"\s+", Whitespace),
            # `///` doc comments and `//` line comments.
            (r"///.*?$", Comment.Special),
            (r"//.*?$", Comment.Single),
            # Block keyword followed by an identifier: `model Question {`.
            (
                r"\b(%s)\b(\s+)([A-Za-z_][\w]*)?"
                % "|".join(_BLOCK_KEYWORDS),
                bygroups(Keyword, Whitespace, Name.Class),
            ),
            # Block-level attributes: `@@index`, `@@unique`, `@@map`.
            (r"@@[A-Za-z_][\w]*", Name.Decorator),
            # Field-level attributes: `@id`, `@version`, `@default`, `@relation`.
            (r"@[A-Za-z_][\w]*", Name.Decorator),
            # Scalar types (with optional `?` / `[]` handled by later rules).
            (words(_SCALAR_TYPES, suffix=r"\b"), Keyword.Type),
            # Capitalised identifiers → model / enum references.
            (r"\b[A-Z][\w]*\b", Name.Class),
            # Strings.
            (r'"(\\.|[^"\\])*"', String.Double),
            (r"'(\\.|[^'\\])*'", String.Single),
            # Numbers.
            (r"\b\d+\.\d+\b", Number.Float),
            (r"\b\d+\b", Number.Integer),
            # Booleans / null-ish literals.
            (words(("true", "false", "null", "now", "autoincrement", "uuid",
                    "cuid", "dbgenerated", "env"), suffix=r"\b"), Keyword.Constant),
            # Assignment and type decorations.
            (r"=", Operator),
            (r"[?\[\]]", Operator),
            (r"[{}(),.:]", Punctuation),
            # Field / argument identifiers.
            (r"[A-Za-z_][\w]*", Name.Attribute),
            (r".", Text),
        ],
    }
