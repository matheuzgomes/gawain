"""Tests for scripts/compare.py, the differential check of gawain against must-annotate.

The predicate tests run classify on real source text, with the diagnostics that the tools report for
it. The end-to-end tests run main on the purpose-built units under tests/python/data. Two of them use
a stub gawain to reach the outputs that the real tools never give for these units.
"""

import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Final, TypeAlias

import pytest

from scripts import compare

REAL_ROOT: Final[Path] = Path(__file__).resolve().parents[2]
GAWAIN_BIN: Final[Path] = Path(os.environ.get("GAWAIN_BIN", REAL_ROOT / "target" / "debug" / "gawain"))
DATA: Final[Path] = Path(__file__).absolute().parent / "data"

GawainDiag: TypeAlias = tuple[int, int, str, str]
MaDiag: TypeAlias = tuple[int, str, str]

BARE_FINAL: Final[str] = "'LIMIT' uses bare Final; write Final[T]"
ANY_REFERENCE: Final[str] = "'Any' is not allowed in type annotations"
BARE_SUPPRESSION: Final[str] = (
    "'gawain: ignore' needs at least one code, e.g. '# gawain: ignore[GW001]'"
)
SELF_ATTRIBUTE: Final[str] = "attribute 'size' is not declared in the body of class 'Box'"
SELF_TARGET_COLUMN: Final[int] = 14
VAR_MISSING: Final[str] = "Variable 'count' missing type annotation"


@pytest.fixture(autouse=True)
def real_gawain(monkeypatch: pytest.MonkeyPatch) -> None:
    """The tests run the gawain binary that GAWAIN_BIN names, or the repository's own build."""
    monkeypatch.setattr(compare, "GAWAIN", GAWAIN_BIN)


@dataclass(frozen=True)
class Case:
    label: str
    predicate: str
    source: str
    gawain: tuple[GawainDiag, ...] = ()
    ma: tuple[MaDiag, ...] = ()
    extra: tuple[tuple[str, str], ...] = ()


def classify_case(case: Case, directory: Path) -> compare.Row:
    """Writes the source of `case`, builds its row from the given diagnostics and classifies it."""
    path: Path = directory / "unit.py"
    path.write_text(case.source, encoding="utf-8")
    for name, text in case.extra:
        (directory / name).parent.mkdir(parents=True, exist_ok=True)
        (directory / name).write_text(text, encoding="utf-8")
    sources: list[compare.SourceFile] = [
        compare.load_source(file, file.relative_to(directory).as_posix())
        for file in sorted(directory.rglob("*.py"))
    ]
    project: compare.Project = compare.build_project(sources)
    sf: compare.SourceFile = project.by_name["unit.py"]
    gawain: list[compare.Diag] = [
        compare.Diag("unit.py", line, col, code, message)
        for line, col, code, message in case.gawain
    ]
    ma: list[compare.Diag] = [
        compare.Diag("unit.py", line, 0, code, message) for line, code, message in case.ma
    ]
    rows: list[compare.Row] = compare.group_rows(gawain, ma)
    assert len(rows) == 1
    row: compare.Row = rows[0]
    compare.classify(row, compare.Ctx(sf, project))
    return row


ACCEPT: Final[tuple[Case, ...]] = (
    Case(
        "bare Final on a module constant",
        "gawain_only_final",
        "from typing import Final\n\nLIMIT: Final = 3\n",
        gawain=((3, 1, "GW011", BARE_FINAL),),
    ),
    Case(
        "untyped module constant",
        "gawain_only_final",
        "BASE_URL: int = 1\n",
        gawain=((1, 1, "GW010", "constant 'BASE_URL' must be annotated as Final[T]"),),
    ),
    Case(
        "Any imported from typing",
        "gawain_only_any",
        "from typing import Any\n\nvalue: Any = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any through a typing module alias",
        "gawain_only_any",
        "import typing as t\n\nvalue: t.Any = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any named in a string annotation",
        "gawain_only_any",
        'from typing import Any\n\nvalue: "Any" = 1\n',
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "suppression without a code",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore\n",
        gawain=((1, 17, "GW900", BARE_SUPPRESSION),),
    ),
    Case(
        "suppression of a code that nothing reports",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore[GW001]\n",
        gawain=((1, 17, "GW901", "unused suppression 'GW001'"),),
    ),
    Case(
        "gawain directive for the family must-annotate reports",
        "suppressed_by_gawain_comment",
        "value = 1  # gawain: ignore[GW001]\n",
        ma=((1, "TF001", "Variable 'value' missing type annotation"),),
    ),
    Case(
        "for target",
        "ma_loop_target",
        "for item in range(2):\n    pass\n",
        ma=((1, "TF005", "Loop variable 'item' missing type annotation"),),
    ),
    Case(
        "name assigned again after its declaration",
        "reassigned_name",
        "count: int = 0\ncount = 1\n",
        ma=((2, "TF001", VAR_MISSING),),
    ),
    Case(
        "TypeVar value",
        "type_declaring_call",
        "from typing import TypeVar\n\nNumber = TypeVar('Number')\n",
        ma=((3, "TF001", "Variable 'Number' missing type annotation"),),
    ),
    Case(
        "member of an Enum",
        "enum_member",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n",
        ma=((5, "TF004", "Class attribute 'RED' missing type annotation"),),
    ),
    Case(
        "underscore in a class body",
        "class_body_dunder_or_underscore",
        "class Box:\n    _ = 1\n",
        ma=((2, "TF004", "Class attribute '_' missing type annotation"),),
    ),
    Case(
        "__init__ with an annotated parameter",
        "init_without_return",
        "class Box:\n    def __init__(self, size: int):\n        pass\n",
        ma=((2, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "attribute declared in the class body",
        "attr_declared_in_class",
        "class Box:\n    size: int\n\n    def __init__(self, size: int) -> None:\n"
        "        self.size = size\n",
        ma=((5, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "attribute declared by a project ancestor",
        "attr_declared_in_ancestor",
        "class Base:\n    size: int\n\n\nclass Box(Base):\n    def __init__(self, size: int) -> None:\n"
        "        self.size = size\n",
        ma=((7, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "attribute with an external ancestor",
        "attr_external_ancestor",
        "class Box(Exception):\n    def __init__(self, size: int) -> None:\n"
        "        self.size = size\n",
        ma=((3, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "method receiver named other than self",
        "receiver_not_named_self",
        "class Box:\n    def size(this) -> int:\n        return 1\n",
        ma=((2, "TF002", "Argument 'this' missing type annotation"),),
    ),
    Case(
        "staticmethod parameter named self",
        "static_self_parameter",
        "class Box:\n    @staticmethod\n    def make(self) -> None:\n        pass\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW002", "parameter 'self' of function 'make' has no type annotation"),),
    ),
    Case(
        "names in a tuple unpacking",
        "unpacked_name",
        "first, second = 1, 2\n",
        ma=(
            (1, "TF001", "Variable 'first' missing type annotation"),
            (1, "TF001", "Variable 'second' missing type annotation"),
        ),
    ),
    Case(
        "four-underscore name",
        "dunder_four_underscores",
        "____ = 1\n",
        gawain=((1, 1, "GW001", "variable '____' has no type annotation"),),
    ),
    Case(
        "variable in a method",
        "ma_class_depth",
        "class Box:\n    def method(self) -> None:\n        value = 1\n",
        gawain=((3, 9, "GW001", "variable 'value' has no type annotation"),),
    ),
    Case(
        "annotated write to an undeclared attribute",
        "attr_annotated_write",
        "class Box:\n    def reset(self) -> None:\n        self.size: int = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "plain write outside __init__",
        "attr_non_init_method",
        "class Box:\n    def reset(self) -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "for target on the receiver",
        "attr_loop_target",
        "class Box:\n    def reset(self) -> None:\n        for self.size in range(2):\n            pass\n",
        gawain=((3, 18, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "unpacked write to the receiver",
        "attr_unpacked_write",
        "class Box:\n    def reset(self) -> None:\n        self.size, self.count = 0, 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
)

REFUSE: Final[tuple[Case, ...]] = (
    Case(
        "Final with a type argument",
        "gawain_only_final",
        "from typing import Final\n\nLIMIT: Final[int] = 3\n",
        gawain=((3, 1, "GW011", BARE_FINAL),),
    ),
    Case(
        "second bare-Final report on the line is not at a target",
        "gawain_only_final",
        "from typing import Final\n\nLIMIT: Final = 3\n",
        gawain=((3, 1, "GW011", BARE_FINAL), (3, 9, "GW011", BARE_FINAL)),
    ),
    Case(
        "GW010 on a lower-case name",
        "gawain_only_final",
        "limit: int = 1\n",
        gawain=((1, 1, "GW010", "constant 'limit' must be annotated as Final[T]"),),
    ),
    Case(
        "column that is not a typing reference",
        "gawain_only_any",
        "value: int = 1\n",
        gawain=((1, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "unused code that the directive does not list",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore[GW001]\n",
        gawain=((1, 17, "GW901", "unused suppression 'GW002'"),),
    ),
    Case(
        "unknown code that the directive does not list",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore[GW001]\n",
        gawain=((1, 17, "GW900", "unknown code 'GW900' in 'gawain: ignore'"),),
    ),
    Case(
        "directive for another family",
        "suppressed_by_gawain_comment",
        "value = 1  # gawain: ignore[GW002]\n",
        ma=((1, "TF001", "Variable 'value' missing type annotation"),),
    ),
    Case(
        "plain assignment on a loop line",
        "ma_loop_target",
        "item = 2\n",
        ma=((1, "TF005", "Loop variable 'item' missing type annotation"),),
    ),
    Case(
        "first binding of a name",
        "reassigned_name",
        "count = 1\n",
        ma=((1, "TF001", VAR_MISSING),),
    ),
    Case(
        "call that declares no type",
        "type_declaring_call",
        "count = make_count()\n",
        ma=((1, "TF001", VAR_MISSING),),
    ),
    Case(
        "class that is not an Enum",
        "enum_member",
        "class Plain:\n    RED = 1\n",
        ma=((2, "TF004", "Class attribute 'RED' missing type annotation"),),
    ),
    Case(
        "ordinary class attribute",
        "class_body_dunder_or_underscore",
        "class Box:\n    value = 1\n",
        ma=((2, "TF004", "Class attribute 'value' missing type annotation"),),
    ),
    Case(
        "__init__ with no annotated parameter",
        "init_without_return",
        "class Box:\n    def __init__(self, size):\n        pass\n",
        ma=((2, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "attribute declared nowhere",
        "attr_declared_in_class",
        "class Box:\n    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((3, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "ancestor that declares another name",
        "attr_declared_in_ancestor",
        "class Base:\n    other: int\n\n\nclass Box(Base):\n    def __init__(self, size: int) -> None:\n"
        "        self.size = size\n",
        ma=((7, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "object is not an external ancestor",
        "attr_external_ancestor",
        "class Box(object):\n    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((3, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "receiver named self",
        "receiver_not_named_self",
        "class Box:\n    def size(self) -> int:\n        return 1\n",
        ma=((2, "TF002", "Argument 'self' missing type annotation"),),
    ),
    Case(
        "staticmethod parameter named value",
        "static_self_parameter",
        "class Box:\n    @staticmethod\n    def make(value) -> None:\n        pass\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW002", "parameter 'value' of function 'make' has no type annotation"),),
    ),
    Case(
        "single plain name",
        "unpacked_name",
        "count = 1\n",
        ma=((1, "TF001", VAR_MISSING),),
    ),
    Case(
        "name with a single leading underscore",
        "dunder_four_underscores",
        "_x = 1\n",
        gawain=((1, 1, "GW001", "variable '_x' has no type annotation"),),
    ),
    Case(
        "module-level variable",
        "ma_class_depth",
        "value = 1\n",
        gawain=((1, 1, "GW001", "variable 'value' has no type annotation"),),
    ),
    Case(
        "plain write, so not annotated",
        "attr_annotated_write",
        "class Box:\n    def reset(self) -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "plain write in __init__",
        "attr_non_init_method",
        "class Box:\n    def __init__(self) -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "plain write, so not a loop target",
        "attr_loop_target",
        "class Box:\n    def reset(self) -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "plain write, so not unpacked",
        "attr_unpacked_write",
        "class Box:\n    def reset(self) -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
)


def wrapped_any(wraps: int) -> str:
    """The text of the string that wraps `Any` in `wraps` layers of quotes."""
    text: str = "Any"
    for _ in range(wraps):
        text = repr(text)
    return text


def binding_form_cases() -> tuple[Case, ...]:
    """Each form of binding that the first binding of a name may take, followed by an assignment."""
    forms: tuple[tuple[str, str], ...] = (
        ("for loop", "for count in range(2):\n    pass\n"),
        ("with item", "with open(path) as count:\n    pass\n"),
        ("walrus", "if (count := 1):\n    pass\n"),
        ("except handler", "try:\n    pass\nexcept ValueError as count:\n    pass\n"),
        ("import", "import count\n"),
        ("from import", "from pkg import count\n"),
        ("function", "def count() -> None:\n    pass\n"),
        ("class", "class count:\n    pass\n"),
        ("type statement", "type count = int\n"),
        ("match capture", "match value:\n    case count:\n        pass\n"),
        ("match star", "match value:\n    case [*count]:\n        pass\n"),
        ("match mapping", "match value:\n    case {'k': 1, **count}:\n        pass\n"),
    )
    cases: list[Case] = []
    for label, form in forms:
        source: str = form + "count = 1\n"
        cases.append(
            Case(
                f"binding by {label}",
                "reassigned_name",
                source,
                ma=((source.count("\n"), "TF001", VAR_MISSING),),
            )
        )
    return tuple(cases)


MORE_ACCEPT: Final[tuple[Case, ...]] = (
    Case(
        "with target on the receiver, with a read beside it",
        "attr_loop_target",
        "class Box:\n    def reset(self) -> None:\n        with cm() as self.size:\n            pass\n",
        gawain=((3, 27, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "read of another attribute beside the write",
        "attr_non_init_method",
        "class Box:\n    def reset(self) -> None:\n        self.size = self.count + 1\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "Any on a later line of a multi-line annotation",
        "gawain_only_any",
        "from typing import Any\n\nvalue: dict[\n    str,\n    Any,\n] = {}\n",
        gawain=((5, 5, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "star import from typing",
        "gawain_only_any",
        "from typing import *\n\nvalue: Any = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "alias of Any",
        "gawain_only_any",
        "from typing import Any as Anything\n\nvalue: Anything = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any in the first argument of a string Annotated",
        "gawain_only_any",
        'from typing import Any\n\nvalue: "Annotated[Any, 1]" = 1\n',
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any in a string nested four strings deep",
        "gawain_only_any",
        f"from typing import Any\n\nvalue: {wrapped_any(4)!r} = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any as a TypeAlias value",
        "gawain_only_any",
        "from typing import Any, TypeAlias\n\nAlias: TypeAlias = Any\n",
        gawain=((3, 20, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any in a type statement",
        "gawain_only_any",
        "from typing import Any\n\ntype Alias = Any\n",
        gawain=((3, 14, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any as the bound of a type parameter",
        "gawain_only_any",
        "from typing import Any\n\nclass Box[T: Any, **P]:\n    pass\n",
        gawain=((3, 14, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "class constant in a class body",
        "gawain_only_final",
        "class Box:\n    LIMIT: int = 1\n",
        gawain=((2, 5, "GW010", "constant 'LIMIT' must be annotated as Final[T]"),),
    ),
    Case(
        "bare Final on an attribute",
        "gawain_only_final",
        "from typing import Final\n\n\nclass Box:\n    def __init__(self) -> None:\n"
        "        self.LIMIT: Final = 1\n",
        gawain=((6, 14, "GW011", "'LIMIT' uses bare Final; write Final[T]"),),
    ),
    Case(
        "directive with an empty entry",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore[GW001, ]\n",
        gawain=((1, 17, "GW901", "unused suppression 'GW001'"),),
    ),
    Case(
        "directive without its closing bracket",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore[GW001\n",
        gawain=((1, 17, "GW900", BARE_SUPPRESSION),),
    ),
    Case(
        "stdlib base class",
        "attr_external_ancestor",
        "import collections\n\n\nclass Box(collections.UserDict):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "base class that is a call",
        "attr_external_ancestor",
        "def make() -> type:\n    return object\n\n\nclass Box(make()):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((7, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "typing member that is not a neutral base",
        "attr_external_ancestor",
        "from typing import NamedTuple\n\n\nclass Box(NamedTuple):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "base reached through a cycle of re-exports",
        "attr_external_ancestor",
        "from alpha import Thing\n\n\nclass Box(Thing):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
        extra=(("alpha.py", "from beta import Thing\n"), ("beta.py", "from alpha import Thing\n")),
    ),
    Case(
        "base reached through an imported module",
        "attr_declared_in_ancestor",
        "import base\n\n\nclass Box(base.Base):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
        extra=(("base.py", "class Base:\n    size: int\n"),),
    ),
    Case(
        "base reached through a name imported from a package",
        "attr_declared_in_ancestor",
        "from pkg import base\n\n\nclass Box(base.Base):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
        extra=(("pkg/__init__.py", ""), ("pkg/base.py", "class Base:\n    size: int\n")),
    ),
    Case(
        "base reached through a submodule import",
        "attr_declared_in_ancestor",
        "import pkg.base\n\n\nclass Box(pkg.base.Base):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
        extra=(("pkg/__init__.py", ""), ("pkg/base.py", "class Base:\n    size: int\n")),
    ),
    Case(
        "subscripted base class",
        "attr_declared_in_ancestor",
        "class Base:\n    size: int\n\n\nclass Box(Base[int]):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((7, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "ancestor reached by two paths",
        "attr_declared_in_ancestor",
        "from a import A\nfrom b import B\n\n\nclass Box(A, B):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((7, "TF004", "Attribute 'self.size' missing type annotation"),),
        extra=(
            ("base.py", "class Base:\n    size: int\n"),
            ("a.py", "from base import Base\n\n\nclass A(Base):\n    pass\n"),
            ("b.py", "from base import Base\n\n\nclass B(Base):\n    pass\n"),
        ),
    ),
    Case(
        "variadic positional parameter annotated",
        "init_without_return",
        "class Box:\n    def __init__(self, *args: int):\n        pass\n",
        ma=((2, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "variadic keyword parameter annotated",
        "init_without_return",
        "class Box:\n    def __init__(self, **options: int):\n        pass\n",
        ma=((2, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "keyword-only parameter annotated",
        "init_without_return",
        "class Box:\n    def __init__(self, *, size: int):\n        pass\n",
        ma=((2, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "starred target in an unpacking",
        "unpacked_name",
        "first, *rest = 1, 2, 3\n",
        ma=((1, "TF001", "Variable 'rest' missing type annotation"),),
    ),
    Case(
        "subscript assignment and a class, a lambda and a comprehension before the reassignment",
        "reassigned_name",
        "@decorate\nclass Helper(Base, metaclass=Meta):\n    pass\n\n\ncount: int = 0\n"
        "items[0] = 1\nhandler = lambda event: event\nvalues = {key: key for key in range(3)}\n"
        "count = 1\n",
        ma=((10, "TF001", VAR_MISSING),),
    ),
    Case(
        "reassignment inside a function, after a nested def",
        "reassigned_name",
        "def outer() -> None:\n    def inner(x: int) -> int:\n        return x\n"
        "    count: int = 0\n    count = 1\n",
        ma=((5, "TF001", VAR_MISSING),),
    ),
    Case(
        "parameter that is reassigned",
        "reassigned_name",
        "def f(count: int) -> None:\n    count = 1\n",
        ma=((2, "TF001", VAR_MISSING),),
    ),
    Case(
        "variadic positional parameter that is reassigned",
        "reassigned_name",
        "def f(*count: int) -> None:\n    count = ()\n",
        ma=((2, "TF001", VAR_MISSING),),
    ),
    Case(
        "variadic keyword parameter that is reassigned",
        "reassigned_name",
        "def f(**count: int) -> None:\n    count = {}\n",
        ma=((2, "TF001", VAR_MISSING),),
    ),
    Case(
        "keyword-only and positional-only parameters that are reassigned",
        "reassigned_name",
        "def f(count: int, /, *, other: int) -> None:\n    count = 1\n",
        ma=((2, "TF001", VAR_MISSING),),
    ),
    Case(
        "global declaration in a function",
        "reassigned_name",
        "def f() -> None:\n    global count\n    count = 1\n",
        ma=((3, "TF001", VAR_MISSING),),
    ),
    Case(
        "base that is a class attribute of a local class",
        "attr_external_ancestor",
        "class Base:\n    class Inner:\n        pass\n\n\nclass Box(Base.Inner):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((8, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "Any as the only argument of a string Annotated",
        "gawain_only_any",
        'from typing import Any\n\nvalue: "Annotated[Any]" = 1\n',
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any through a typing module alias inside a string",
        "gawain_only_any",
        'import typing as t\n\nvalue: "t.Any" = 1\n',
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "diamond of two classes that share a base",
        "attr_declared_in_ancestor",
        "from a import A\nfrom b import B\n\n\nclass Box(A, B):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((7, "TF004", "Attribute 'self.size' missing type annotation"),),
        extra=(
            ("base.py", "class Base:\n    size: int\n"),
            ("a.py", "from base import Base\n\n\nclass A(Base):\n    pass\n"),
            ("b.py", "from base import Base\n\n\nclass B(Base):\n    pass\n"),
        ),
    ),
) + binding_form_cases()

MORE_REFUSE: Final[tuple[Case, ...]] = (
    Case(
        "lambda parameter named other than self",
        "receiver_not_named_self",
        "handler = lambda this: this\n",
        ma=((1, "TF002", "Argument 'this' missing type annotation"),),
    ),
    Case(
        "read on the right of an assignment",
        "attr_non_init_method",
        "class Box:\n    def reset(self) -> None:\n        value = self.size\n",
        gawain=((3, 22, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "string annotation that does not parse, naming Any",
        "gawain_only_any",
        'from typing import Any\n\nvalue: "Any(" = 1\n',
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any inside a Literal",
        "gawain_only_any",
        "from typing import Any, Literal\n\nvalue: Literal[Any] = 1\n",
        gawain=((3, 16, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any in the metadata of an Annotated",
        "gawain_only_any",
        "from typing import Annotated, Any\n\nvalue: Annotated[int, Any] = 1\n",
        gawain=((3, 23, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Literal in a string annotation",
        "gawain_only_any",
        "from typing import Any\n\nvalue: \"Literal['Any']\" = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any in a string nested five strings deep",
        "gawain_only_any",
        f"from typing import Any\n\nvalue: {wrapped_any(5)!r} = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "a constant that is not a string",
        "gawain_only_any",
        "value: 3 = 1\n",
        gawain=((1, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "a string annotation that does not parse",
        "gawain_only_final",
        'from typing import Final\n\nLIMIT: "Final(" = 3\n',
        gawain=((3, 1, "GW011", BARE_FINAL),),
    ),
    Case(
        "typing module attribute other than Any",
        "gawain_only_any",
        "import typing as t\n\nvalue: t.Dict = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any imported from a module that is not typing",
        "gawain_only_any",
        "from os import Any\n\nvalue: Any = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "star import from a module that is not typing",
        "gawain_only_any",
        "from os import *\n\nvalue: Any = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any rebound by a module-level import after a typing star",
        "gawain_only_any",
        "from typing import *\nfrom os import Any\n\nvalue: Any = 1\n",
        gawain=((4, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "final on a subscript target",
        "gawain_only_final",
        "from typing import Final\n\nitems[0]: Final = 1\n",
        gawain=((3, 1, "GW011", BARE_FINAL),),
    ),
    Case(
        "comment that is not a gawain directive",
        "suppressed_by_gawain_comment",
        "value = 1  # noqa\n",
        ma=((1, "TF001", "Variable 'value' missing type annotation"),),
    ),
    Case(
        "write through another object's attribute",
        "attr_non_init_method",
        "class Box:\n    def reset(self, other: 'Box') -> None:\n        other.size = 0\n",
        gawain=((3, 15, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "attribute write at module level",
        "attr_non_init_method",
        "obj.size = 0\n",
        gawain=((1, 5, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "attribute write in a method without a receiver",
        "attr_non_init_method",
        "class Box:\n    def reset() -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
    Case(
        "column of no parameter",
        "static_self_parameter",
        "class Box:\n    @staticmethod\n    def make(self) -> None:\n        pass\n",
        gawain=((3, 1, "GW002", "parameter 'self' of function 'make' has no type annotation"),),
    ),
    Case(
        "self parameter of a method that is not static",
        "static_self_parameter",
        "class Box:\n    def make(self) -> None:\n        pass\n",
        gawain=((2, SELF_TARGET_COLUMN, "GW002", "parameter 'self' of function 'make' has no type annotation"),),
    ),
    Case(
        "receiver of a function outside a class",
        "receiver_not_named_self",
        "def size(this) -> int:\n    return 1\n",
        ma=((1, "TF002", "Argument 'this' missing type annotation"),),
    ),
    Case(
        "second parameter of a method",
        "receiver_not_named_self",
        "class Box:\n    def size(self, this) -> int:\n        return 1\n",
        ma=((2, "TF002", "Argument 'this' missing type annotation"),),
    ),
    Case(
        "class base that is object's Protocol",
        "attr_external_ancestor",
        "import typing\n\n\nclass Box(typing.Protocol):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((5, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "binding that comes after the assignment",
        "reassigned_name",
        "count = 1\nfor count in range(2):\n    pass\n",
        ma=((1, "TF001", VAR_MISSING),),
    ),
)

EXTRA_ACCEPT: Final[tuple[Case, ...]] = (
    Case(
        "one-letter alias of Any, which ends where the annotation ends",
        "gawain_only_any",
        "from typing import Any as A\n\nvalue: A = 1\n",
        gawain=((3, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any inside the type argument of an Annotated",
        "gawain_only_any",
        "from typing import Annotated, Any\n\nvalue: Annotated[list[Any], 1] = []\n",
        gawain=((3, 23, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "Any imported only inside a function",
        "gawain_only_any",
        "def f() -> None:\n    from typing import Any\n\n\nvalue: Any = 1\n",
        gawain=((5, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "static __init__ whose first parameter is annotated",
        "init_without_return",
        "class Box:\n    @staticmethod\n    def __init__(size: int):\n        pass\n",
        ma=((3, "TF003", "Function '__init__' missing return type annotation"),),
    ),
)

EXTRA_REFUSE: Final[tuple[Case, ...]] = (
    Case(
        "four underscores in a class body",
        "class_body_dunder_or_underscore",
        "class Box:\n    ____ = 1\n",
        ma=((2, "TF004", "Class attribute '____' missing type annotation"),),
    ),
    Case(
        "private name with a leading double underscore",
        "class_body_dunder_or_underscore",
        "class Box:\n    __private = 1\n",
        ma=((2, "TF004", "Class attribute '__private' missing type annotation"),),
    ),
    Case(
        "name that ends with two underscores only",
        "dunder_four_underscores",
        "x__ = 1\n",
        gawain=((1, 1, "GW001", "variable 'x__' has no type annotation"),),
    ),
    Case(
        "bare TypeAlias declaration without a value",
        "gawain_only_any",
        "from typing import Any, TypeAlias\n\nAlias: TypeAlias\nvalue: int = 1\n",
        gawain=((4, 8, "GW020", ANY_REFERENCE),),
    ),
    Case(
        "constant already typed as Final",
        "gawain_only_final",
        "BASE_URL: Final[int] = 1\n",
        gawain=((1, 1, "GW010", "constant 'BASE_URL' must be annotated as Final[T]"),),
    ),
    Case(
        "constant declared as a TypeAlias",
        "gawain_only_final",
        "BASE_URL: TypeAlias = int\n",
        gawain=((1, 1, "GW010", "constant 'BASE_URL' must be annotated as Final[T]"),),
    ),
    Case(
        "__init__ outside a class",
        "init_without_return",
        "def __init__(self, size: int):\n    pass\n",
        ma=((1, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "plain name beside an unpacking on the same line",
        "unpacked_name",
        "count = first, second = 1, 2\n",
        ma=((1, "TF001", "Variable 'count' missing type annotation"),),
    ),
    Case(
        "receiver annotated, with no other parameter annotated",
        "init_without_return",
        "class Box:\n    def __init__(self: 'Box', size):\n        pass\n",
        ma=((2, "TF003", "Function '__init__' missing return type annotation"),),
    ),
    Case(
        "directive with an empty list and another message",
        "gawain_only_suppression",
        "value: int = 1  # gawain: ignore[]\n",
        gawain=((1, 17, "GW900", "gawain says something else"),),
    ),
    Case(
        "class attribute with no receiver",
        "attr_declared_in_class",
        "class Box:\n    size = 1\n",
        ma=((2, "TF004", "Class attribute 'size' missing type annotation"),),
    ),
    Case(
        "Generic base is neutral, not an external ancestor",
        "attr_external_ancestor",
        "from typing import Generic\n\n\nclass Box(Generic):\n"
        "    def __init__(self, size: int) -> None:\n        self.size = size\n",
        ma=((6, "TF004", "Attribute 'self.size' missing type annotation"),),
    ),
    Case(
        "Enum name that nothing imports",
        "enum_member",
        "class Color(Enum):\n    RED = 1\n",
        ma=((2, "TF004", "Class attribute 'RED' missing type annotation"),),
    ),
    Case(
        "external ancestor leaves the attribute undeclared",
        "attr_non_init_method",
        "class Box(Exception):\n    def reset(self) -> None:\n        self.size = 0\n",
        gawain=((3, SELF_TARGET_COLUMN, "GW004", SELF_ATTRIBUTE),),
    ),
)

ALL_ACCEPT: Final[tuple[Case, ...]] = ACCEPT + MORE_ACCEPT + EXTRA_ACCEPT
ALL_REFUSE: Final[tuple[Case, ...]] = REFUSE + MORE_REFUSE + EXTRA_REFUSE

@pytest.mark.parametrize("case", ALL_ACCEPT, ids=[case.label for case in ALL_ACCEPT])
def test_predicate_accepts_its_case(case: Case, tmp_path: Path) -> None:
    row: compare.Row = classify_case(case, tmp_path)
    assert row.status == "expected"
    assert row.explained_by == case.predicate
    assert row.reason == next(e.reason for e in compare.EXPECTED if e.name == case.predicate)


@pytest.mark.parametrize("case", ALL_REFUSE, ids=[case.label for case in ALL_REFUSE])
def test_predicate_refuses_its_look_alike(case: Case, tmp_path: Path) -> None:
    row: compare.Row = classify_case(case, tmp_path)
    assert row.explained_by != case.predicate


def test_every_expected_predicate_has_an_accepting_and_a_refusing_case() -> None:
    accepted: set[str] = {case.predicate for case in ALL_ACCEPT}
    refused: set[str] = {case.predicate for case in ALL_REFUSE}
    names: set[str] = {expectation.name for expectation in compare.EXPECTED}
    assert accepted == names
    assert refused == names


def test_a_key_that_both_tools_report_agrees(tmp_path: Path) -> None:
    case: Case = Case(
        "both",
        "",
        "count = 0\n",
        gawain=((1, 1, "GW001", "variable 'count' has no type annotation"),),
        ma=((1, "TF001", VAR_MISSING),),
    )
    row: compare.Row = classify_case(case, tmp_path)
    assert row.status == "agree"
    assert row.explained_by == ""


def test_a_key_that_no_predicate_explains_is_unexplained(tmp_path: Path) -> None:
    case: Case = Case(
        "nothing",
        "",
        "VALUE: int = 1\n",
        gawain=((1, 1, "GW003", "function 'f' has no return type annotation"),),
    )
    row: compare.Row = classify_case(case, tmp_path)
    assert row.status == "unexplained"
    assert row.reason == "no EXPECTED predicate explains this"


# --- Helpers that build their input from real files ----------------------------------------------


def test_units_of_takes_python_files_and_directories_in_name_order(tmp_path: Path) -> None:
    (tmp_path / "b_dir").mkdir()
    (tmp_path / "c.py").write_text("", encoding="utf-8")
    (tmp_path / "a.py").write_text("", encoding="utf-8")
    (tmp_path / "notes.txt").write_text("", encoding="utf-8")
    assert [unit.name for unit in compare.units_of(tmp_path)] == ["a.py", "b_dir", "c.py"]


@pytest.mark.parametrize(
    ("module", "is_package", "target", "level", "expected"),
    [
        ("pkg.sub", False, "base", 1, "pkg.base"),
        ("a.b.c", False, "d", 2, "a.d"),
        ("a.b", True, None, 1, "a.b"),
        ("a.b", True, "c", 2, "a.c"),
        ("", False, "base", 1, "base"),
        ("x", False, None, 0, ""),
        ("x", False, "os", 0, "os"),
        ("a.b", False, "x", 3, "x"),
    ],
)
def test_absolute_module_resolves_relative_imports(
    module: str, is_package: bool, target: str | None, level: int, expected: str
) -> None:
    assert compare.absolute_module(module, is_package, target, level) == expected


def test_module_name_follows_package_directories(tmp_path: Path) -> None:
    package: Path = tmp_path / "pkg"
    sub: Path = package / "sub"
    sub.mkdir(parents=True)
    (package / "__init__.py").write_text("", encoding="utf-8")
    (sub / "__init__.py").write_text("", encoding="utf-8")
    (sub / "leaf.py").write_text("", encoding="utf-8")
    (tmp_path / "loose.py").write_text("", encoding="utf-8")
    assert compare.module_name_of(package / "__init__.py") == "pkg"
    assert compare.module_name_of(sub / "leaf.py") == "pkg.sub.leaf"
    assert compare.module_name_of(tmp_path / "loose.py") == "loose"


def test_a_key_reported_by_both_tools_is_neither_only_gawain_nor_only_ma() -> None:
    rows: list[compare.Row] = compare.group_rows(
        [compare.Diag("a.py", 1, 1, "GW001", "m")], [compare.Diag("a.py", 1, 0, "TF001", "m")]
    )
    assert len(rows) == 1
    assert not rows[0].only_gawain
    assert not rows[0].only_ma


def test_a_key_reported_by_must_annotate_alone_is_only_ma() -> None:
    rows: list[compare.Row] = compare.group_rows([], [compare.Diag("a.py", 1, 0, "TF001", "m")])
    assert rows[0].only_ma
    assert not rows[0].only_gawain


def test_default_root_is_the_fixture_corpus() -> None:
    assert compare.parse_args([]).root == str(compare.REPO_ROOT / "tests" / "fixtures")


def test_char_column_counts_characters_not_bytes() -> None:
    assert compare.char_column(["é = 1"], 1, 3) == 2


def test_group_rows_keys_on_name_line_and_family() -> None:
    gawain: list[compare.Diag] = [compare.Diag("a.py", 1, 1, "GW001", "m")]
    ma: list[compare.Diag] = [
        compare.Diag("a.py", 1, 0, "TF001", "m"),
        compare.Diag("a.py", 2, 0, "TF001", "m"),
    ]
    rows: list[compare.Row] = compare.group_rows(gawain, ma)
    assert [(row.line, len(row.gawain), len(row.ma)) for row in rows] == [(1, 1, 1), (2, 0, 1)]


def test_render_table_pads_columns_and_leaves_no_trailing_space() -> None:
    row: compare.Row = compare.Row("a.py", 1, "VAR")
    row.gawain.append(compare.Diag("a.py", 1, 1, "GW001", "m"))
    row.status = "agree"
    lines: list[str] = compare.render_table([row])
    assert [line.split() for line in lines] == [
        ["file", "line", "gawain", "must-annotate", "status", "reason"],
        ["a.py", "1", "GW001", "-", "agree"],
    ]
    assert all(line == line.rstrip() for line in lines)


# --- End to end ----------------------------------------------------------------------------------


def table_of(out: str) -> list[list[str]]:
    """The table rows of the printed report, each split into its columns."""
    rows: list[list[str]] = []
    for line in out.splitlines():
        if not line:
            break
        rows.append(re.split(r"\s{2,}", line.strip()))
    return rows


def write_stub(directory: Path, body: str, exit_code: int = 0) -> Path:
    """A gawain stand-in that prints the JSON that `body` evaluates to, for the unit it is given."""
    stub: Path = directory / "gawain-stub"
    stub.write_text(
        f"#!{sys.executable}\nimport json\nimport os\nimport sys\n\nunit = sys.argv[-1]\n"
        f"json.dump({body}, sys.stdout)\nsys.exit({exit_code})\n",
        encoding="utf-8",
    )
    stub.chmod(0o755)
    return stub


def test_report_lists_agree_and_expected_rows(capsys: pytest.CaptureFixture[str]) -> None:
    code: int = compare.main([str(DATA / "report")])
    out: str = capsys.readouterr().out
    assert code == 0
    rows: list[list[str]] = table_of(out)
    assert rows[0][:5] == ["file", "line", "gawain", "must-annotate", "status"]
    assert rows[1][:5] == ["agree.py", "1", "GW001", "TF001", "agree"]
    assert rows[2][:5] == ["finals.py", "3", "GW011", "-", "expected"]
    assert rows[3][:5] == ["finals.py", "4", "GW020", "-", "expected"]
    assert rows[2][5].startswith("gawain only: must-annotate has no Final rule")
    lines: list[str] = out.splitlines()
    assert "1 agree · 2 expected · 0 unexplained" in lines
    assert "     1  gawain_only_final: " + compare.EXPECTED[0].reason in lines
    assert "     1  gawain_only_any: " + compare.EXPECTED[1].reason in lines
    for expectation in compare.EXPECTED:
        assert f"  {expectation.name}: {expectation.reason}" in out


def test_hide_agree_leaves_agreeing_rows_out_of_the_table(capsys: pytest.CaptureFixture[str]) -> None:
    code: int = compare.main(["--hide-agree", str(DATA / "report")])
    out: str = capsys.readouterr().out
    assert code == 0
    assert [row[0] for row in table_of(out)] == ["file", "finals.py", "finals.py"]
    assert "1 agree · 2 expected · 0 unexplained" in out.splitlines()


def test_package_resolves_inheritance_across_modules(capsys: pytest.CaptureFixture[str]) -> None:
    code: int = compare.main([str(DATA / "inherit")])
    rows: list[list[str]] = table_of(capsys.readouterr().out)
    assert code == 0
    assert rows[1][:5] == ["package/box.py", "6", "-", "TF004", "expected"]
    assert rows[1][5].startswith("self.x is declared by a project ancestor")


def test_gawain_row_no_predicate_explains_exits_one(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    stub: Path = write_stub(
        tmp_path,
        '{"presence": [{"path": unit, "line": 1, "col": 1, "code": "GW003", '
        '"message": "function \'f\' has no return type annotation"}], "types": []}',
    )
    monkeypatch.setattr(compare, "GAWAIN", stub)
    code: int = compare.main([str(DATA / "unexplained")])
    out: str = capsys.readouterr().out
    assert code == 1
    assert table_of(out)[1] == [
        "unit.py",
        "1",
        "GW003",
        "-",
        "unexplained",
        "no EXPECTED predicate explains this",
    ]
    assert "0 agree · 0 expected · 1 unexplained" in out.splitlines()


@pytest.mark.parametrize(
    ("body", "exit_code", "message"),
    [
        ("[]", 0, "gawain printed no JSON object"),
        ('{"presence": 3}', 0, "gawain presence is not a JSON array"),
        ('{"presence": [7]}', 0, "gawain presence holds an item that is not a JSON object"),
        (
            '{"presence": [{"path": unit, "line": "1", "col": 1, "code": "GW001", "message": "m"}]}',
            0,
            "expected a number for 'line'",
        ),
        (
            '{"presence": [{"path": unit, "line": 1, "col": True, "code": "GW001", "message": "m"}]}',
            0,
            "expected a number for 'col'",
        ),
        (
            '{"presence": [{"path": unit, "line": 1, "col": 1, "code": 7, "message": "m"}]}',
            0,
            "expected text for 'code'",
        ),
        (
            '{"presence": [{"path": unit, "line": 1, "col": 1, "code": "XX999", "message": "m"}]}',
            0,
            "unknown code 'XX999'",
        ),
        (
            '{"presence": [{"path": "/nonexistent/x.py", "line": 1, "col": 1, "code": "GW001", '
            '"message": "m"}]}',
            0,
            "is outside",
        ),
        (
            '{"presence": [{"path": unit, "line": 1, "col": 1, "code": "GW001"}]}',
            0,
            "expected text for 'message'",
        ),
        ("{\"presence\": []}", 3, "gawain exited with 3 on unit.py"),
    ],
)
def test_gawain_output_it_cannot_read_is_a_tool_error(
    body: str,
    exit_code: int,
    message: str,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(compare, "GAWAIN", write_stub(tmp_path, body, exit_code))
    code: int = compare.main([str(DATA / "unexplained")])
    assert code == 2
    assert message in capsys.readouterr().err


def test_gawain_diagnostic_for_a_file_outside_the_unit_is_a_tool_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    (tmp_path / "unit.py").write_text("VALUE: int = 1\n", encoding="utf-8")
    (tmp_path / "other.py").write_text("VALUE: int = 1\n", encoding="utf-8")
    stub: Path = write_stub(
        tmp_path,
        '{"presence": [{"path": os.path.join(os.path.dirname(unit), "other.py"), "line": 1, '
        '"col": 1, "code": "GW001", "message": "m"}]}',
    )
    monkeypatch.setattr(compare, "GAWAIN", stub)
    code: int = compare.main([str(tmp_path)])
    assert code == 2
    assert "other.py has a diagnostic but is not part of unit.py" in capsys.readouterr().err


def test_gawain_stderr_is_shown_as_text_in_the_tool_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    stub: Path = tmp_path / "gawain-stub"
    stub.write_text(
        f"#!{sys.executable}\nimport sys\n\nsys.stderr.write('boom\\n')\nsys.exit(3)\n",
        encoding="utf-8",
    )
    stub.chmod(0o755)
    monkeypatch.setattr(compare, "GAWAIN", stub)
    code: int = compare.main([str(DATA / "unexplained")])
    assert code == 2
    assert "compare: gawain exited with 3 on unit.py: boom\n" in capsys.readouterr().err


def test_missing_gawain_binary_is_a_tool_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(compare, "GAWAIN", tmp_path / "no-such-gawain")
    code: int = compare.main([str(DATA / "report")])
    assert code == 2
    assert "gawain binary not found" in capsys.readouterr().err


def test_must_annotate_exit_status_is_a_tool_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    failing: Path = tmp_path / "failing-python"
    failing.write_text("#!/bin/sh\nexit 4\n", encoding="utf-8")
    failing.chmod(0o755)
    monkeypatch.setattr(sys, "executable", str(failing))
    code: int = compare.main([str(DATA / "report")])
    assert code == 2
    assert "must-annotate exited with 4 on agree.py" in capsys.readouterr().err


def test_root_that_is_not_a_directory_is_a_tool_error(capsys: pytest.CaptureFixture[str]) -> None:
    code: int = compare.main([str(DATA / "report" / "agree.py")])
    assert code == 2
    assert "is not a directory" in capsys.readouterr().err


def test_directory_without_python_units_prints_only_the_summary(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (tmp_path / "notes.txt").write_text("", encoding="utf-8")
    code: int = compare.main([str(tmp_path)])
    out: str = capsys.readouterr().out
    assert code == 0
    assert table_of(out) == [["file", "line", "gawain", "must-annotate", "status", "reason"]]
    assert "0 agree · 0 expected · 0 unexplained" in out.splitlines()
