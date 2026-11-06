"""mkdocs hook: register the DORM ``.schema`` Pygments lexer for the docs build.

Wired via ``hooks:`` in ``mkdocs.yml``. Registering here (rather than shipping a
packaged Pygments entry point) keeps the lexer self-contained to the docs tree.

``pymdownx.highlight`` binds ``get_lexer_by_name`` at import time, so to be
robust against import order we patch the reference in *both* ``pygments.lexers``
and ``pymdownx.highlight`` to resolve the ``schema`` / ``dorm`` aliases to our
lexer, delegating everything else to the original resolver.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(__file__))

from dorm_lexer import DormSchemaLexer  # noqa: E402

_ALIASES = {"dorm", "dormschema", "dorm-schema", "schema"}


def _install():
    import pygments.lexers as pl

    original = pl.get_lexer_by_name

    def resolve(name, **options):
        if name in _ALIASES:
            return DormSchemaLexer(**options)
        return original(name, **options)

    pl.get_lexer_by_name = resolve

    # pymdownx.highlight captured its own reference with `from ... import`.
    try:
        import pymdownx.highlight as ph

        ph.get_lexer_by_name = resolve
    except Exception:  # pragma: no cover - pymdownx always present in CI
        pass


_install()


def on_config(config):
    # Re-install in case the Pygments/pymdownx modules were (re)imported after
    # this hook module was first loaded (e.g. under `mkdocs serve` reloads).
    _install()
    return config
