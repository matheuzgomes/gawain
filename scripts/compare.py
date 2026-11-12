# /// script
# requires-python = ">=3.14"
# dependencies = ["must-annotate==0.1.2"]
# ///
"""Differential test: gawain against must-annotate 0.1.2 on gawain's fixtures.

Run from the repository root, after `cargo build`:

    uv run scripts/compare.py tests/fixtures

Each unit is a top-level .py file or a subdirectory of the root, as in tests/fixtures.rs. Both tools
check every unit. Their diagnostics are normalised to (file, line, family). A key that both tools
report agrees. A key that only one tool reports is expected when an EXPECTED predicate explains it,
and unexplained otherwise. The script exits 1 when any key is unexplained, and 2 when a tool cannot
run.
"""

import argparse
import ast
import io
import json
import re
import subprocess
import sys
import tempfile
import tokenize
from collections import Counter
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Final, NoReturn, TypeAlias, TypeIs

REPO_ROOT: Final[Path] = Path(__file__).resolve().parent.parent
GAWAIN: Final[Path] = REPO_ROOT / "target" / "debug" / "gawain"
# exclude = [] matters: must-annotate's default exclusion of "tests/" would skip every fixture.
# strict = true turns on TF005, which is off by default.
MUST_ANNOTATE_CONFIG: Final[str] = '[tool.must-annotate]\nstrict = true\nexclude = []\n'
# The column of a must-annotate diagnostic. must-annotate gives none, and no predicate reads one.
NO_COLUMN: Final[int] = 0

FAMILY_OF_CODE: Final[dict[str, str]] = {
    "TF001": "VAR",
    "TF002": "PARAM",
    "TF003": "RETURN",
    "TF004": "ATTR",
    "TF005": "LOOP",
    "GW001": "VAR",
    "GW002": "PARAM",
    "GW003": "RETURN",
    "GW004": "ATTR",
    "GW010": "FINAL",
    "GW011": "FINAL",
    "GW020": "ANY",
    "GW900": "SUPPRESSION",
    "GW901": "SUPPRESSION",
}
# The gawain codes that a `gawain: ignore[...]` directive can silence, per family.
GAWAIN_CODES_BY_FAMILY: Final[dict[str, frozenset[str]]] = {
    "VAR": frozenset({"GW001"}),
    "PARAM": frozenset({"GW002"}),
    "RETURN": frozenset({"GW003"}),
    "ATTR": frozenset({"GW004"}),
    "FINAL": frozenset({"GW010", "GW011"}),
    "ANY": frozenset({"GW020"}),
    "LOOP": frozenset(),
    "SUPPRESSION": frozenset(),
}

# gawain's directive grammar: `gawain`, optional spaces, a colon, `ignore` as a whole word. The word
# `gawain` must follow `#` or whitespace.
DIRECTIVE_RE: Final[re.Pattern[str]] = re.compile(r"(?<=[#\s])gawain\s*:\s*ignore(?![A-Za-z0-9_])")
DIRECTIVE_CODES_RE: Final[re.Pattern[str]] = re.compile(r"\s*\[([^\]]*)\]")
QUOTED_RE: Final[re.Pattern[str]] = re.compile(r"'([^']*)'")
CONSTANT_RE: Final[re.Pattern[str]] = re.compile(r"_*[A-Z][A-Z0-9_]*")
# Codes that a suppression can silence: every gawain code except GW900.
SUPPRESSIBLE_CODES: Final[frozenset[str]] = frozenset().union(*GAWAIN_CODES_BY_FAMILY.values())
NEEDS_CODE_PREFIX: Final[str] = "'gawain: ignore' needs at least one code"
UNUSED_PREFIX: Final[str] = "unused suppression '"
TYPING_MODULES: Final[frozenset[str]] = frozenset({"typing", "typing_extensions"})
MAX_STRING_DEPTH: Final[int] = 4

TYPE_DECLARING_CALLS: Final[frozenset[str]] = frozenset(
    {"TypeVar", "ParamSpec", "TypeVarTuple", "NewType", "NamedTuple", "TypedDict"}
)
ENUM_FAMILY: Final[frozenset[str]] = frozenset(
    {"Enum", "IntEnum", "StrEnum", "Flag", "IntFlag", "ReprEnum"}
)
LOOP_NODES: Final[tuple[type[ast.AST], ...]] = (ast.For, ast.AsyncFor, ast.With, ast.AsyncWith)
FUNCTION_NODES: Final[tuple[type[ast.FunctionDef], type[ast.AsyncFunctionDef]]] = (
    ast.FunctionDef,
    ast.AsyncFunctionDef,
)
SCOPE_NODES: Final[tuple[type[ast.AST], ...]] = (
    ast.Module,
    ast.ClassDef,
    ast.FunctionDef,
    ast.AsyncFunctionDef,
    ast.Lambda,
)

# A node that carries a position, and a name bound at a position.
Positioned: TypeAlias = ast.stmt | ast.expr | ast.arg | ast.alias | ast.excepthandler | ast.pattern
NamePos: TypeAlias = tuple[str, tuple[int, int]]
# The scopes that hold statements: a module, a class, or a function.
Scope: TypeAlias = ast.Module | ast.ClassDef | ast.FunctionDef | ast.AsyncFunctionDef


class CompareError(Exception):
    """The comparison cannot finish: a tool is missing or printed something unexpected."""


@dataclass(frozen=True)
class Diag:
    """One diagnostic from either tool. `name` is the file relative to the root. `col` is gawain's
    1-based character column, and 0 for must-annotate, whose column is not used."""

    name: str
    line: int
    col: int
    code: str
    message: str

    @property
    def family(self) -> str:
        return FAMILY_OF_CODE[self.code]


@dataclass
class Row:
    """All the diagnostics of one (file, line, family) key."""

    name: str
    line: int
    family: str
    gawain: list[Diag] = field(default_factory=list)
    ma: list[Diag] = field(default_factory=list)
    status: str = ""
    reason: str = ""
    explained_by: str = ""

    @property
    def only_gawain(self) -> bool:
        return bool(self.gawain) and not self.ma

    @property
    def only_ma(self) -> bool:
        return bool(self.ma) and not self.gawain


# --- Running the tools -------------------------------------------------------------------------


def relative_name(path: Path, root: Path) -> str:
    try:
        return path.resolve().relative_to(root).as_posix()
    except ValueError as error:
        raise CompareError(f"{path} is outside {root}") from error


def make_diag(path: Path, root: Path, line: int, col: int, code: str, message: str) -> Diag:
    if code not in FAMILY_OF_CODE:
        raise CompareError(f"unknown code {code!r} for {path}")
    return Diag(relative_name(path, root), line, col, code, message)


def _refuse(item: dict[str, object], key: str, what: str) -> NoReturn:
    raise CompareError(f"expected {what} for {key!r} in {item!r}")


def text_field(item: dict[str, object], key: str) -> str:
    value: object = item.get(key)
    return value if isinstance(value, str) else _refuse(item, key, "text")


def int_field(item: dict[str, object], key: str) -> int:
    value: object = item.get(key)
    return value if isinstance(value, int) and not isinstance(value, bool) else _refuse(item, key, "a number")


def json_objects(payload: object, what: str) -> list[dict[str, object]]:
    if not isinstance(payload, list):
        raise CompareError(f"{what} is not a JSON array")
    objects: list[dict[str, object]] = []
    for item in payload:
        if not isinstance(item, dict):
            raise CompareError(f"{what} holds an item that is not a JSON object")
        objects.append(item)
    return objects


def run_tool(name: str, command: list[str], unit: Path, ok_codes: tuple[int, ...]) -> str:
    """The standard output of a tool that checked `unit`. An exit status outside `ok_codes` is a
    CompareError, because the tool cannot be trusted with this unit."""
    completed: subprocess.CompletedProcess[str] = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    if completed.returncode not in ok_codes:
        raise CompareError(
            f"{name} exited with {completed.returncode} on {unit.name}: {completed.stderr.strip()}"
        )
    return completed.stdout


def run_gawain(unit: Path, root: Path) -> list[Diag]:
    """gawain's presence diagnostics for `unit`, in strict mode, which is the mode that must-annotate
    checks. Run from the repository root, so that gawain reads its own pyproject.toml the way the cargo
    tests do. Exit status 1 means that gawain found problems."""
    command: list[str] = [
        str(GAWAIN),
        "check",
        "--no-types",
        "--format",
        "json",
        "--mode",
        "strict",
        str(unit),
    ]
    ok_codes: tuple[int, ...] = (0, 1)
    stdout: str = run_tool("gawain", command, unit, ok_codes)
    report: object = json.loads(stdout)
    if not isinstance(report, dict):
        raise CompareError("gawain printed no JSON object")
    return [
        make_diag(
            REPO_ROOT / text_field(item, "path"),
            root,
            int_field(item, "line"),
            int_field(item, "col"),
            text_field(item, "code"),
            text_field(item, "message"),
        )
        for item in json_objects(report.get("presence"), "gawain presence")
    ]


def run_must_annotate(unit: Path, root: Path, config_dir: Path) -> list[Diag]:
    """must-annotate's diagnostics for `unit`, read from its JSON output. `--config` points at a
    temporary pyproject.toml, so the repository's own pyproject is left alone."""
    command: list[str] = [
        sys.executable,
        "-m",
        "must_annotate",
        "check",
        str(unit),
        "--format",
        "json",
        "--config",
        str(config_dir),
    ]
    ok_codes: tuple[int, ...] = (0,)
    stdout: str = run_tool("must-annotate", command, unit, ok_codes)
    payload: object = json.loads(stdout)
    return [
        make_diag(
            Path(text_field(item, "file")),
            root,
            int_field(item, "line"),
            NO_COLUMN,
            text_field(item, "code"),
            text_field(item, "message"),
        )
        for item in json_objects(payload, "must-annotate output")
    ]


# --- Source model --------------------------------------------------------------------------------


def char_column(lines: list[str], line: int, byte_col: int) -> int:
    """The 0-based character column of an ast byte column on `line`."""
    prefix: bytes = lines[line - 1].encode("utf-8")[:byte_col]
    return len(prefix.decode("utf-8", errors="replace"))


def position(node: Positioned) -> tuple[int, int]:
    return (node.lineno, node.col_offset)


def module_name_of(path: Path) -> str:
    """The dotted module name, as gawain names it: the stem, then each parent directory that holds an
    __init__.py."""
    parts: list[str] = [] if path.name == "__init__.py" else [path.stem]
    directory: Path = path.parent
    while (directory / "__init__.py").is_file():
        parts.insert(0, directory.name)
        directory = directory.parent
    return ".".join(parts)


def dotted_parts(expr: ast.expr) -> list[str] | None:
    if isinstance(expr, ast.Name):
        return [expr.id]
    if isinstance(expr, ast.Attribute):
        return _extend(dotted_parts(expr.value), expr.attr)
    return None


def call_name(expr: ast.expr) -> str | None:
    if isinstance(expr, ast.Name):
        return expr.id
    if isinstance(expr, ast.Attribute):
        return expr.attr
    return None


def ends_with_name(expr: ast.expr | None, name: str) -> bool:
    return call_name(expr) == name if expr is not None else False


def is_staticmethod(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> bool:
    return any(ends_with_name(decorator, "staticmethod") for decorator in fn.decorator_list)


def raw_first_positional(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> ast.arg | None:
    params: list[ast.arg] = [*fn.args.posonlyargs, *fn.args.args]
    return params[0] if params else None


def first_positional(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> ast.arg | None:
    """The receiver of a method, as gawain takes it: the first positional parameter, unless the
    method is a staticmethod."""
    return None if is_staticmethod(fn) else raw_first_positional(fn)


def _judged_without_receiver(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> list[ast.arg]:
    positional: list[ast.arg] = [*fn.args.posonlyargs, *fn.args.args]
    return positional[1:] if positional and not is_staticmethod(fn) else positional


def _variadic_params(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> list[ast.arg]:
    return [special for special in (fn.args.vararg, fn.args.kwarg) if special is not None]


def judged_params(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> list[ast.arg]:
    """The parameters of a method in a class body that gawain judges: all but the receiver."""
    return [*_judged_without_receiver(fn), *fn.args.kwonlyargs, *_variadic_params(fn)]


def parameters(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> list[ast.arg]:
    """Every parameter of a function, in the order that gawain binds them."""
    found: list[ast.arg] = [*fn.args.posonlyargs, *fn.args.args, *fn.args.kwonlyargs]
    if fn.args.vararg is not None:
        found.append(fn.args.vararg)
    if fn.args.kwarg is not None:
        found.append(fn.args.kwarg)
    return found


def arg_owner(sf: "SourceFile", arg: ast.arg) -> ast.FunctionDef | ast.AsyncFunctionDef | None:
    owner: ast.AST | None = sf.parent(arg)
    owner = sf.parent(owner) if owner is not None else None
    if isinstance(owner, (ast.FunctionDef, ast.AsyncFunctionDef)):
        return owner
    return None


def declared_names(cls: ast.ClassDef) -> set[str]:
    """The names that the class body annotates, such as `x` in `x: int`."""
    names: set[str] = set()
    for stmt in cls.body:
        if isinstance(stmt, ast.AnnAssign) and isinstance(stmt.target, ast.Name):
            names.add(stmt.target.id)
    return names


def is_dunder(name: str) -> bool:
    """gawain's dunder: begins and ends with two underscores and is longer than four characters."""
    return len(name) > 4 and name.startswith("__") and name.endswith("__")


def is_short_dunder(name: str) -> bool:
    """Begins and ends with two underscores but is at most four characters: `____`, which
    must-annotate skips and gawain reports."""
    return len(name) <= 4 and name.startswith("__") and name.endswith("__")


def is_constant(name: str) -> bool:
    """gawain's constant: UPPER_CASE after leading underscores, and not a dunder."""
    return CONSTANT_RE.fullmatch(name) is not None and not is_dunder(name)


def annotation_expr(annotation: ast.expr) -> ast.expr | None:
    """The expression an annotation stands for. A string annotation stands for what its content
    parses to, and nothing when the content does not parse."""
    if isinstance(annotation, ast.Constant) and isinstance(annotation.value, str):
        try:
            parsed: ast.Expression = ast.parse(annotation.value, mode="eval")
        except SyntaxError:
            return None
        return parsed.body
    return annotation


def is_bare_final(annotation: ast.expr) -> bool:
    return ends_with_name(annotation_expr(annotation), "Final")


def is_typed_final(annotation: ast.expr) -> bool:
    expr: ast.expr | None = annotation_expr(annotation)
    return isinstance(expr, ast.Subscript) and ends_with_name(expr.value, "Final")


def is_type_alias(annotation: ast.expr) -> bool:
    return ends_with_name(annotation_expr(annotation), "TypeAlias")


def _extend(inner: list[str] | None, attr: str) -> list[str] | None:
    return None if inner is None else [*inner, attr]


def target_names(target: ast.expr, unpacked: bool = False) -> list[tuple[ast.Name, bool]]:
    """The names that an assignment target binds, each with whether it sits inside an unpacking."""
    if isinstance(target, ast.Name):
        return [(target, unpacked)]
    if isinstance(target, (ast.Tuple, ast.List)):
        return [pair for element in target.elts for pair in target_names(element, True)]
    if isinstance(target, ast.Starred):
        return target_names(target.value, True)
    return []


def bound_name(dotted: str, asname: str | None) -> str:
    if asname is not None:
        return asname
    return dotted.split(".", 1)[0]


def bindings_in(target: ast.expr) -> list[NamePos]:
    """The names an assignment target binds, at the positions of those names."""
    return [(name.id, position(name)) for name, _unpacked in target_names(target)]


# How each statement or expression binds names in its own scope, keyed by its exact type. A
# comprehension and a lambda bind nothing in the enclosing scope, so they have no entry.


def _def_names(node: ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef, _in: bool) -> list[NamePos]:
    return [(node.name, position(node))]


def _assign_names(node: ast.Assign, _in: bool) -> list[NamePos]:
    return [pair for target in node.targets for pair in bindings_in(target)]


def _annotated_names(node: ast.AnnAssign, _in: bool) -> list[NamePos]:
    return [(node.target.id, position(node.target))] if isinstance(node.target, ast.Name) else []


def _loop_names(node: ast.For | ast.AsyncFor, _in: bool) -> list[NamePos]:
    return bindings_in(node.target)


def _with_names(node: ast.With | ast.AsyncWith, _in: bool) -> list[NamePos]:
    return [
        pair
        for item in node.items
        if item.optional_vars is not None
        for pair in bindings_in(item.optional_vars)
    ]


def _walrus_names(node: ast.NamedExpr, _in: bool) -> list[NamePos]:
    return [(node.target.id, position(node.target))] if isinstance(node.target, ast.Name) else []


def _except_names(node: ast.ExceptHandler, _in: bool) -> list[NamePos]:
    return [] if node.name is None else [(node.name, position(node))]


def _import_names(node: ast.Import | ast.ImportFrom, _in: bool) -> list[NamePos]:
    return [(bound_name(alias.name, alias.asname), position(node)) for alias in node.names]


def _global_names(node: ast.Global | ast.Nonlocal, in_function: bool) -> list[NamePos]:
    return [(name, position(node)) for name in node.names] if in_function else []


def _type_alias_names(node: ast.TypeAlias, _in: bool) -> list[NamePos]:
    return [(node.name.id, position(node))] if isinstance(node.name, ast.Name) else []


def _match_names(node: ast.MatchAs | ast.MatchStar, _in: bool) -> list[NamePos]:
    return [] if node.name is None else [(node.name, position(node))]


def _mapping_names(node: ast.MatchMapping, _in: bool) -> list[NamePos]:
    return [] if node.rest is None else [(node.rest, position(node))]


_OWN_NAMES: Final[dict[type[ast.AST], Callable[..., list[NamePos]]]] = {
    ast.FunctionDef: _def_names,
    ast.AsyncFunctionDef: _def_names,
    ast.ClassDef: _def_names,
    ast.Assign: _assign_names,
    ast.AnnAssign: _annotated_names,
    ast.For: _loop_names,
    ast.AsyncFor: _loop_names,
    ast.With: _with_names,
    ast.AsyncWith: _with_names,
    ast.NamedExpr: _walrus_names,
    ast.ExceptHandler: _except_names,
    ast.Import: _import_names,
    ast.ImportFrom: _import_names,
    ast.Global: _global_names,
    ast.Nonlocal: _global_names,
    ast.TypeAlias: _type_alias_names,
    ast.MatchAs: _match_names,
    ast.MatchStar: _match_names,
    ast.MatchMapping: _mapping_names,
}


def own_names(node: ast.AST, in_function: bool) -> list[NamePos]:
    """The names that `node` binds in the scope around it."""
    handler: Callable[..., list[NamePos]] | None = _OWN_NAMES.get(type(node))
    return [] if handler is None else handler(node, in_function)


def _definition_children(
    node: ast.FunctionDef | ast.AsyncFunctionDef,
) -> list[ast.AST]:
    """A nested def contributes its decorators, parameters and return annotation, not its body."""
    children: list[ast.AST] = [*node.decorator_list, node.args]
    if node.returns is not None:
        children.append(node.returns)
    return children


def _class_children(node: ast.ClassDef) -> list[ast.AST]:
    return [*node.decorator_list, *node.bases, *node.keywords]


def _lambda_children(node: ast.Lambda) -> list[ast.AST]:
    return [node.args]


def _comprehension_children(
    node: ast.ListComp | ast.SetComp | ast.GeneratorExp | ast.DictComp,
) -> list[ast.AST]:
    """A comprehension's own targets are local to it. Its iterables, conditions and result count."""
    heads: list[ast.AST] = [node.key, node.value] if isinstance(node, ast.DictComp) else [node.elt]
    iterables: list[ast.AST] = [
        child for generator in node.generators for child in (generator.iter, *generator.ifs)
    ]
    return [*iterables, *heads]


_SCOPE_CHILDREN: Final[dict[type[ast.AST], Callable[..., list[ast.AST]]]] = {
    ast.FunctionDef: _definition_children,
    ast.AsyncFunctionDef: _definition_children,
    ast.ClassDef: _class_children,
    ast.Lambda: _lambda_children,
    ast.ListComp: _comprehension_children,
    ast.SetComp: _comprehension_children,
    ast.GeneratorExp: _comprehension_children,
    ast.DictComp: _comprehension_children,
}


def scope_children(node: ast.AST) -> list[ast.AST]:
    """The children of `node` whose bindings count in the enclosing scope."""
    handler: Callable[..., list[ast.AST]] | None = _SCOPE_CHILDREN.get(type(node))
    return list(ast.iter_child_nodes(node)) if handler is None else handler(node)


def collect_bindings(node: ast.AST, in_function: bool, found: list[NamePos]) -> None:
    """Appends the names that `node` binds in the scope around it, as gawain counts bindings."""
    found.extend(own_names(node, in_function))
    for child in scope_children(node):
        collect_bindings(child, in_function, found)


def scope_bindings(scope: Scope) -> list[NamePos]:
    """Every name bound in `scope` with the position of the binding, in the order gawain would see
    them. A function's parameters are bound on entry, so they come first."""
    found: list[NamePos] = []
    in_function: bool = isinstance(scope, FUNCTION_NODES)
    if isinstance(scope, FUNCTION_NODES):
        found.extend((param.arg, position(param)) for param in parameters(scope))
    for stmt in scope.body:
        collect_bindings(stmt, in_function, found)
    return found


def _function_annotations(node: ast.FunctionDef | ast.AsyncFunctionDef) -> list[ast.expr]:
    found: list[ast.expr] = [param.annotation for param in parameters(node) if param.annotation]
    if node.returns is not None:
        found.append(node.returns)
    return [*found, *type_param_exprs(node)]


def _class_annotations(node: ast.ClassDef) -> list[ast.expr]:
    return type_param_exprs(node)


def _assignment_annotations(node: ast.AnnAssign) -> list[ast.expr]:
    if node.value is not None and is_type_alias(node.annotation):
        return [node.annotation, node.value]
    return [node.annotation]


def _alias_annotations(node: ast.TypeAlias) -> list[ast.expr]:
    return [node.value, *type_param_exprs(node)]


_ANNOTATIONS: Final[dict[type[ast.AST], Callable[..., list[ast.expr]]]] = {
    ast.FunctionDef: _function_annotations,
    ast.AsyncFunctionDef: _function_annotations,
    ast.ClassDef: _class_annotations,
    ast.AnnAssign: _assignment_annotations,
    ast.TypeAlias: _alias_annotations,
}


def annotation_nodes(tree: ast.Module) -> list[ast.expr]:
    """The expressions that gawain reads as types: parameter and return annotations, annotated
    assignment annotations, TypeAlias values, type-parameter bounds and defaults, and `type` values."""
    found: list[ast.expr] = []
    for node in ast.walk(tree):
        handler: Callable[..., list[ast.expr]] | None = _ANNOTATIONS.get(type(node))
        if handler is not None:
            found.extend(handler(node))
    return found


def _type_param_parts(type_param: ast.AST) -> list[ast.expr | None]:
    """The bound and the default of a type parameter. Only a TypeVar has a bound."""
    if isinstance(type_param, ast.TypeVar):
        return [type_param.bound, type_param.default_value]
    return [getattr(type_param, "default_value", None)]


def type_param_exprs(node: ast.AST) -> list[ast.expr]:
    found: list[ast.expr] = []
    for type_param in getattr(node, "type_params", []):
        found.extend(part for part in _type_param_parts(type_param) if part is not None)
    return found


@dataclass(frozen=True)
class TypingNames:
    """The names that a file binds to typing.Any, to a typing module, or to neither ("other"), as gawain
    resolves them. A module-level import wins over a nested one, and a module-level star import provides a
    bare Any that no import binds."""

    kinds: dict[str, str]
    star_any: bool

    def is_any(self, name: str) -> bool:
        if name in self.kinds:
            return self.kinds[name] == "any"
        return self.star_any and name == "Any"

    def is_module(self, name: str) -> bool:
        return self.kinds.get(name) == "module"


def _import_kind(node: ast.Import | ast.ImportFrom, alias: ast.alias) -> tuple[str, str] | None:
    """The name that one alias binds, with its kind. A star import binds no name."""
    if isinstance(node, ast.ImportFrom):
        return _from_import_kind(node, alias)
    bound: str = alias.asname or alias.name.split(".")[0]
    return (bound, _module_kind(alias.name))


def _module_kind(name: str) -> str:
    return "module" if name in TYPING_MODULES else "other"


def _from_import_kind(node: ast.ImportFrom, alias: ast.alias) -> tuple[str, str] | None:
    if alias.name == "*":
        return None
    return (alias.asname or alias.name, _from_kind(node, alias))


def _from_kind(node: ast.ImportFrom, alias: ast.alias) -> str:
    is_any: bool = node.module in TYPING_MODULES and alias.name == "Any"
    return "any" if is_any else "other"


def _import_kinds(node: ast.Import | ast.ImportFrom) -> list[tuple[str, str]]:
    """Each name an import binds, with its kind."""
    kinds: list[tuple[str, str] | None] = [_import_kind(node, alias) for alias in node.names]
    return [pair for pair in kinds if pair is not None]


def _star_any(tree: ast.Module, module_level: dict[str, str]) -> bool:
    """Whether a module-level star import from typing provides a bare Any that no import binds."""
    return any(_is_typing_star(stmt) for stmt in tree.body) and "Any" not in module_level


def typing_names_of(tree: ast.Module) -> TypingNames:
    entries: list[tuple[str, str, bool]] = _typing_bindings(tree)
    module_level: dict[str, str] = {bound: kind for bound, kind, is_top in entries if is_top}
    nested: dict[str, str] = {bound: kind for bound, kind, is_top in entries if not is_top}
    return TypingNames(kinds={**nested, **module_level}, star_any=_star_any(tree, module_level))


def _typing_bindings(tree: ast.Module) -> list[tuple[str, str, bool]]:
    """Each name that an import binds in the file: the name, its kind, and whether the import sits at
    module level."""
    top_level: set[int] = {id(stmt) for stmt in tree.body}
    return [
        (bound, kind, id(node) in top_level)
        for node in ast.walk(tree)
        if isinstance(node, (ast.Import, ast.ImportFrom))
        for bound, kind in _import_kinds(node)
    ]


def _is_typing_star(node: ast.stmt) -> bool:
    return (
        isinstance(node, ast.ImportFrom)
        and node.module in TYPING_MODULES
        and any(alias.name == "*" for alias in node.names)
    )


@dataclass
class SourceFile:
    """One analysed file with the indexes that the predicates read."""

    path: Path
    name: str
    module: str
    is_package: bool
    lines: list[str]
    tree: ast.Module
    parents: dict[ast.AST, ast.AST]
    by_line: dict[int, list[ast.AST]]
    comments: dict[int, list[str]]
    classes: dict[str, ast.ClassDef]
    imports: dict[str, tuple[str, str | None]]
    annotation_spans: list[tuple[tuple[int, int], tuple[int, int]]]
    typing_names: TypingNames
    bindings: dict[ast.AST, list[NamePos]] = field(default_factory=dict)

    def nodes_on(self, line: int) -> list[ast.AST]:
        return self.by_line.get(line, [])

    def start_col(self, node: ast.expr | ast.arg) -> int:
        return char_column(self.lines, node.lineno, node.col_offset)

    def attr_col(self, node: ast.Attribute) -> int:
        """The 0-based character column of the attribute name, which ends the node."""
        assert node.end_lineno is not None
        assert node.end_col_offset is not None
        return char_column(self.lines, node.end_lineno, node.end_col_offset) - len(node.attr)

    def parent(self, node: ast.AST) -> ast.AST | None:
        return self.parents.get(node)

    def ancestors(self, node: ast.AST) -> Iterator[ast.AST]:
        current: ast.AST | None = self.parents.get(node)
        while current is not None:
            yield current
            current = self.parents.get(current)

    def scope_of(self, node: ast.AST) -> ast.AST:
        """The nearest enclosing scope: module, class, function or lambda. Every node in the file is in
        the module, so the module is the fallback."""
        return next(
            (ancestor for ancestor in self.ancestors(node) if isinstance(ancestor, SCOPE_NODES)),
            self.tree,
        )

    def bindings_of(self, scope: Scope) -> list[NamePos]:
        cached: list[NamePos] | None = self.bindings.get(scope)
        if cached is None:
            cached = scope_bindings(scope)
            self.bindings[scope] = cached
        return cached


def index_nodes(tree: ast.Module) -> tuple[dict[ast.AST, ast.AST], dict[int, list[ast.AST]]]:
    """The parent of each node, and the nodes that start on each line."""
    parents: dict[ast.AST, ast.AST] = {}
    by_line: dict[int, list[ast.AST]] = {}
    for node in ast.walk(tree):
        for child in ast.iter_child_nodes(node):
            parents[child] = node
        lineno: object = getattr(node, "lineno", None)
        if isinstance(lineno, int):
            by_line.setdefault(lineno, []).append(node)
    return parents, by_line


def comment_lines(text: str) -> dict[int, list[str]]:
    comments: dict[int, list[str]] = {}
    for token in tokenize.generate_tokens(io.StringIO(text).readline):
        if token.type == tokenize.COMMENT:
            comments.setdefault(token.start[0], []).append(token.string)
    return comments


def _import_entries(
    stmt: ast.Import | ast.ImportFrom, module: str, is_package: bool
) -> list[tuple[str, tuple[str, str | None]]]:
    """Each name an import binds, with the module it comes from and the member it names, if any."""
    if isinstance(stmt, ast.Import):
        return [
            (
                bound_name(alias.name, alias.asname),
                (alias.name if alias.asname else bound_name(alias.name, None), None),
            )
            for alias in stmt.names
        ]
    base: str = absolute_module(module, is_package, stmt.module, stmt.level)
    return [(alias.asname or alias.name, (base, alias.name)) for alias in stmt.names]


def _bind_statement(
    stmt: ast.stmt,
    module: str,
    is_package: bool,
    classes: dict[str, ast.ClassDef],
    imports: dict[str, tuple[str, str | None]],
) -> None:
    """Records what one module-level statement binds, as gawain reads the module."""
    if isinstance(stmt, ast.ClassDef):
        classes[stmt.name] = stmt
    elif isinstance(stmt, (ast.Import, ast.ImportFrom)):
        for bound, binding in _import_entries(stmt, module, is_package):
            imports[bound] = binding
            classes.pop(bound, None)


def module_bindings(
    tree: ast.Module, module: str, is_package: bool
) -> tuple[dict[str, ast.ClassDef], dict[str, tuple[str, str | None]]]:
    """The module-level classes and imports. The last module-level binding of a name wins, as in
    gawain: a later class replaces an earlier class, and a later import takes the name from a class."""
    classes: dict[str, ast.ClassDef] = {}
    imports: dict[str, tuple[str, str | None]] = {}
    for stmt in tree.body:
        _bind_statement(stmt, module, is_package, classes, imports)
    return classes, imports


def annotation_spans(
    tree: ast.Module, lines: list[str]
) -> list[tuple[tuple[int, int], tuple[int, int]]]:
    spans: list[tuple[tuple[int, int], tuple[int, int]]] = []
    for expr in annotation_nodes(tree):
        assert expr.end_lineno is not None
        assert expr.end_col_offset is not None
        start: tuple[int, int] = (expr.lineno, char_column(lines, expr.lineno, expr.col_offset))
        end: tuple[int, int] = (
            expr.end_lineno,
            char_column(lines, expr.end_lineno, expr.end_col_offset),
        )
        spans.append((start, end))
    return spans


def load_source(path: Path, name: str) -> SourceFile:
    text: str = path.read_text(encoding="utf-8")
    tree: ast.Module = ast.parse(text, filename=str(path))
    lines: list[str] = text.split("\n")
    module: str = module_name_of(path)
    is_package: bool = path.name == "__init__.py"
    parents, by_line = index_nodes(tree)
    classes, imports = module_bindings(tree, module, is_package)
    return SourceFile(
        path=path,
        name=name,
        module=module,
        is_package=is_package,
        lines=lines,
        tree=tree,
        parents=parents,
        by_line=by_line,
        comments=comment_lines(text),
        classes=classes,
        imports=imports,
        annotation_spans=annotation_spans(tree, lines),
        typing_names=typing_names_of(tree),
    )


def _package_part(module: str, is_package: bool, level: int) -> str:
    """The package that a relative import of `level` dots starts from."""
    parts: list[str] = module.split(".") if module else []
    package: list[str] = parts if is_package else parts[:-1]
    kept: int = max(0, len(package) - (level - 1))
    return ".".join(package[:kept])


def _join_target(base: str, target: str | None) -> str:
    if not target:
        return base
    return f"{base}.{target}" if base else target


def absolute_module(module: str, is_package: bool, target: str | None, level: int) -> str:
    """The module that an import statement names, with relative levels resolved."""
    if level == 0:
        return target or ""
    return _join_target(_package_part(module, is_package, level), target)


# --- Class facts ---------------------------------------------------------------------------------


@dataclass(frozen=True)
class Ref:
    """What a name refers to. `kind` is class, module, member, unbound, neutral, enum or external."""

    kind: str
    module: str = ""
    name: str = ""


@dataclass(frozen=True)
class Lineage:
    """What gawain's attribute rule reads from a class: the names it declares with its ancestors, and
    whether an external ancestor or an Enum ancestor is among them."""

    declared: frozenset[str]
    external: bool
    enum: bool


def _newly_visited(ref: Ref, visited: set[tuple[str, str]]) -> bool:
    """Whether `ref` names a project class that the walk has not expanded yet."""
    key: tuple[str, str] = (ref.module, ref.name)
    if ref.kind != "class" or key in visited:
        return False
    visited.add(key)
    return True


def is_neutral(ref: Ref) -> bool:
    """`object`, `Generic` and `Protocol` are bases that gawain ignores."""
    if ref.kind == "unbound":
        return ref.name == "object"
    return ref.kind == "member" and ref.module in TYPING_MODULES and ref.name in ("Generic", "Protocol")


def is_enum_base(ref: Ref) -> bool:
    return ref.kind == "member" and ref.module == "enum" and ref.name in ENUM_FAMILY


@dataclass
class Project:
    """The files of one unit, indexed for resolving bases across modules."""

    by_module: dict[str, SourceFile]
    by_name: dict[str, SourceFile]

    def lookup(self, module: str, name: str, seen: frozenset[tuple[str, str]]) -> Ref:
        owner: SourceFile = self.by_module[module]
        if name in owner.classes:
            return Ref("class", module, name)
        binding: tuple[str, str | None] | None = owner.imports.get(name)
        if binding is None:
            return Ref("unbound", name=name)
        target, member = binding
        if member is None:
            return Ref("module", target)
        return self.imported(target, member, seen)

    def imported(self, target: str, member: str, seen: frozenset[tuple[str, str]]) -> Ref:
        """What `from target import member` names: a submodule, or a member of the target module."""
        candidate: str = f"{target}.{member}" if target else member
        if candidate in self.by_module:
            return Ref("module", candidate)
        if target not in self.by_module:
            return Ref("member", target, member)
        return self.through(target, member, seen)

    def step(self, ref: Ref, part: str) -> Ref:
        if ref.kind != "module":
            return Ref("external")
        if ref.module not in self.by_module:
            return Ref("member", ref.module, part)
        return self.submodule_or_member(ref.module, part)

    def resolve(self, sf: SourceFile, expr: ast.expr) -> Ref:
        if isinstance(expr, ast.Subscript):
            expr = expr.value
        parts: list[str] | None = dotted_parts(expr)
        if parts is None:
            return Ref("external")
        ref: Ref = self.lookup(sf.module, parts[0], frozenset())
        for part in parts[1:]:
            ref = self.step(ref, part)
        return ref

    def classify(self, sf: SourceFile, base: ast.expr) -> Ref:
        ref: Ref = self.resolve(sf, base)
        if ref.kind == "class":
            return ref
        if is_neutral(ref):
            return Ref("neutral")
        if is_enum_base(ref):
            return Ref("enum")
        return Ref("external")

    def lineage(self, sf: SourceFile, cls: ast.ClassDef) -> Lineage:
        """Walks the ancestors of `cls` as gawain does, and returns the names they declare, and whether an
        external or an Enum ancestor is among them."""
        refs: list[Ref] = self.ancestry(sf, cls.bases, set())
        declared: set[str] = declared_names(cls)
        for ref in refs:
            if ref.kind == "class":
                declared |= declared_names(self.class_of(ref))
        kinds: set[str] = {ref.kind for ref in refs}
        return Lineage(frozenset(declared), "external" in kinds, "enum" in kinds)

    def through(self, target: str, member: str, seen: frozenset[tuple[str, str]]) -> Ref:
        """The member of a project module, followed through the modules that import it in turn."""
        key: tuple[str, str] = (target, member)
        if key in seen:
            return Ref("external")
        return self.lookup(target, member, seen | {key})

    def submodule_or_member(self, module: str, part: str) -> Ref:
        submodule: str = f"{module}.{part}"
        if submodule in self.by_module:
            return Ref("module", submodule)
        return self.lookup(module, part, frozenset())

    def class_of(self, ref: Ref) -> ast.ClassDef:
        return self.by_module[ref.module].classes[ref.name]

    def ancestry(
        self, sf: SourceFile, bases: list[ast.expr], visited: set[tuple[str, str]]
    ) -> list[Ref]:
        """What each base names, and what each project class among them inherits from. A class is
        expanded once, whichever way it is reached."""
        found: list[Ref] = []
        for base in bases:
            ref: Ref = self.classify(sf, base)
            found.append(ref)
            if _newly_visited(ref, visited):
                owner: SourceFile = self.by_module[ref.module]
                found.extend(self.ancestry(owner, self.class_of(ref).bases, visited))
        return found


def build_project(sources: list[SourceFile]) -> Project:
    return Project(
        by_module={source.module: source for source in sources},
        by_name={source.name: source for source in sources},
    )


# --- Receiver writes -----------------------------------------------------------------------------


@dataclass(frozen=True)
class Write:
    """An attribute bound through a method's receiver, such as `self.x = 1`. `kind` is plain (a `=`
    assignment), annotated (`self.x: T`), loop (a for or with target) or unpacked (inside a tuple)."""

    kind: str
    method: ast.FunctionDef | ast.AsyncFunctionDef
    cls: ast.ClassDef
    attr: str
    col: int


def _annotated_kind(holder: ast.AnnAssign, top: ast.AST, _attribute: ast.Attribute) -> str | None:
    return "annotated" if holder.target is top else None


def _assign_kind(holder: ast.Assign, top: ast.AST, attribute: ast.Attribute) -> str | None:
    if not any(target is top for target in holder.targets):
        return None
    return "plain" if top is attribute else "unpacked"


def _loop_kind(holder: ast.For | ast.AsyncFor, top: ast.AST, _attribute: ast.Attribute) -> str | None:
    return "loop" if holder.target is top else None


def _with_kind(holder: ast.withitem, top: ast.AST, _attribute: ast.Attribute) -> str | None:
    return "loop" if holder.optional_vars is top else None


def _holder_kind(holder: ast.AST | None, top: ast.AST, attribute: ast.Attribute) -> str | None:
    """How the statement that holds `top` stores into `attribute`, or None when it does not store."""
    table: dict[type[ast.AST], Callable[..., str | None]] = {
        ast.AnnAssign: _annotated_kind,
        ast.Assign: _assign_kind,
        ast.For: _loop_kind,
        ast.AsyncFor: _loop_kind,
        ast.withitem: _with_kind,
    }
    handler: Callable[..., str | None] | None = table.get(type(holder))
    return None if handler is None else handler(holder, top, attribute)


def store_kind(sf: SourceFile, attribute: ast.Attribute) -> str | None:
    top: ast.AST = attribute
    holder: ast.AST | None = sf.parent(top)
    while isinstance(holder, (ast.Tuple, ast.List, ast.Starred)):
        top = holder
        holder = sf.parent(top)
    return _holder_kind(holder, top, attribute)


def method_and_class(
    sf: SourceFile, attribute: ast.Attribute
) -> tuple[ast.FunctionDef | ast.AsyncFunctionDef, ast.ClassDef] | None:
    """The method that holds `attribute`, and the class that holds the method."""
    method: ast.AST | None = sf.scope_of(attribute)
    if not isinstance(method, FUNCTION_NODES):
        return None
    cls: ast.AST | None = sf.scope_of(method)
    return (method, cls) if isinstance(cls, ast.ClassDef) else None


def _is_receiver_write(method: ast.FunctionDef | ast.AsyncFunctionDef, attribute: ast.Attribute) -> bool:
    """Whether the attribute is written through the first positional parameter of `method`."""
    receiver: ast.arg | None = first_positional(method)
    return (
        receiver is not None
        and isinstance(attribute.value, ast.Name)
        and attribute.value.id == receiver.arg
    )


def _receiver_store_kind(
    sf: SourceFile, method: ast.FunctionDef | ast.AsyncFunctionDef, attribute: ast.Attribute
) -> str | None:
    return store_kind(sf, attribute) if _is_receiver_write(method, attribute) else None


def receiver_write(sf: SourceFile, attribute: ast.Attribute) -> Write | None:
    owner: tuple[ast.FunctionDef | ast.AsyncFunctionDef, ast.ClassDef] | None = method_and_class(
        sf, attribute
    )
    if owner is None:
        return None
    method, cls = owner
    kind: str | None = _receiver_store_kind(sf, method, attribute)
    if kind is None:
        return None
    return Write(kind, method, cls, attribute.attr, sf.attr_col(attribute))


def receiver_writes_on(sf: SourceFile, line: int) -> list[Write]:
    writes: list[Write] = []
    for node in sf.nodes_on(line):
        if isinstance(node, ast.Attribute):
            write: Write | None = receiver_write(sf, node)
            if write is not None:
                writes.append(write)
    return writes


def assigns_on(sf: SourceFile, line: int) -> list[ast.Assign]:
    return [node for node in sf.nodes_on(line) if isinstance(node, ast.Assign)]


def direct_assigns(sf: SourceFile, line: int, name: str) -> list[ast.Assign]:
    """The assignments on `line` with `name` as a plain target, such as `x = 1` or `x = y = 1`."""
    return [
        assign
        for assign in assigns_on(sf, line)
        if any(isinstance(target, ast.Name) and target.id == name for target in assign.targets)
    ]


def assigned_names(sf: SourceFile, line: int) -> list[tuple[str, bool]]:
    return [
        (name.id, unpacked)
        for assign in assigns_on(sf, line)
        for target in assign.targets
        for name, unpacked in target_names(target)
    ]


# --- Diagnostic facts ----------------------------------------------------------------------------


def name_of(diag: Diag) -> str:
    """The first quoted name in a message, or an empty string when there is none."""
    match: re.Match[str] | None = QUOTED_RE.search(diag.message)
    return match.group(1) if match else ""


def directive_entries(sf: SourceFile, line: int) -> list[str] | None:
    """The entries that the first gawain directive on `line` lists, trimmed and without empty ones. None
    when the line has no directive. A directive without a closed pair of brackets lists none."""
    for comment in sf.comments.get(line, []):
        entries: list[str] | None = _directive_entries_in(comment)
        if entries is not None:
            return entries
    return None


def _unknown_listed(diag: Diag, entries: list[str]) -> bool:
    entry: str = name_of(diag)
    return entry in entries and entry not in SUPPRESSIBLE_CODES


def _malformed_with(diag: Diag, entries: list[str]) -> bool:
    if diag.message.startswith("unknown code '"):
        return _unknown_listed(diag, entries)
    return diag.message.startswith(NEEDS_CODE_PREFIX) and not entries


def gawain_malformed(sf: SourceFile, diag: Diag) -> bool:
    """Whether a GW900 is the report that gawain makes for the directive on its line, as its message
    says: an entry that is not a suppressible code, or a directive that lists no code."""
    entries: list[str] | None = directive_entries(sf, diag.line)
    return entries is not None and _malformed_with(diag, entries)


def _listed_suppressible(sf: SourceFile, diag: Diag) -> bool:
    entries: list[str] | None = directive_entries(sf, diag.line)
    code: str = name_of(diag)
    return entries is not None and code in entries and code in SUPPRESSIBLE_CODES


def gawain_unused(sf: SourceFile, diag: Diag) -> bool:
    """Whether a GW901 is the report that gawain makes for a suppressible code that the directive on its
    line lists, as its message says. must-annotate has no equivalent, because it does not read gawain's
    directives."""
    return (
        diag.code == "GW901"
        and diag.message.startswith(UNUSED_PREFIX)
        and _listed_suppressible(sf, diag)
    )


def first_argument(subscript: ast.Subscript) -> ast.expr:
    """The type that a subscript stands for: the first element of a tuple, or the whole slice."""
    annotated: ast.expr = subscript.slice
    if isinstance(annotated, ast.Tuple) and annotated.elts:
        return annotated.elts[0]
    return annotated


def subscript_named(node: ast.AST, name: str) -> bool:
    """Whether `node` is a subscript of a type with the given name, such as Literal[...]."""
    return isinstance(node, ast.Subscript) and ends_with_name(node.value, name)


def _type_children(node: ast.AST) -> list[ast.AST]:
    """The children that gawain reads as types. The arguments of Literal[...] are not types, and the
    metadata of Annotated[...] is not one either."""
    if subscript_named(node, "Literal"):
        return []
    if subscript_named(node, "Annotated"):
        assert isinstance(node, ast.Subscript)
        return [node.value, first_argument(node)]
    return list(ast.iter_child_nodes(node))


def type_nodes(node: ast.AST) -> Iterator[ast.AST]:
    """The nodes of `node` that gawain reads as types, the same as those of a type expression."""
    yield node
    for child in _type_children(node):
        yield from type_nodes(child)


def _excludes(ancestor: ast.AST, node: ast.AST, chain: list[ast.AST]) -> bool:
    if not isinstance(ancestor, ast.Subscript):
        return False
    if ends_with_name(ancestor.value, "Literal"):
        return True
    return ends_with_name(ancestor.value, "Annotated") and _in_metadata(ancestor, node, chain)


def _in_metadata(ancestor: ast.Subscript, node: ast.AST, chain: list[ast.AST]) -> bool:
    """Whether `node` is in the metadata of an Annotated subscript, rather than in its type."""
    first: ast.expr = first_argument(ancestor)
    return first is not node and first not in chain


def excluded_from_types(sf: SourceFile, node: ast.AST) -> bool:
    """Whether `node` sits in Literal[...] or in the metadata of Annotated[...], so gawain does not read
    it as a type."""
    ancestors: list[ast.AST] = list(sf.ancestors(node))
    chain: list[ast.AST] = [node, *ancestors]
    return any(_excludes(ancestor, node, chain) for ancestor in ancestors)


def is_typing_attribute(node: ast.Attribute, names: TypingNames) -> bool:
    return node.attr == "Any" and isinstance(node.value, ast.Name) and names.is_module(node.value.id)


def string_mentions_any(text: str, names: TypingNames, depth: int) -> bool:
    """Whether the content of a string annotation names typing.Any, reading nested strings too."""
    if depth > MAX_STRING_DEPTH:
        return False
    try:
        parsed: ast.Expression = ast.parse(text, mode="eval")
    except SyntaxError:
        return False
    return any(
        _type_node_is_any(node, names, depth) for node in type_nodes(parsed.body)
    )


def _type_node_is_any(node: ast.AST, names: TypingNames, depth: int) -> bool:
    if isinstance(node, ast.Name):
        return names.is_any(node.id)
    if isinstance(node, ast.Attribute):
        return is_typing_attribute(node, names)
    return _nested_string_mentions_any(node, names, depth)


def _reference_candidate(sf: SourceFile, node: ast.AST, column: int) -> bool:
    """Whether `node` could be the reference that gawain reports at `column`."""
    return (
        isinstance(node, (ast.Name, ast.Attribute, ast.Constant))
        and sf.start_col(node) == column
        and not excluded_from_types(sf, node)
    )


def _nested_string_mentions_any(node: ast.AST, names: TypingNames, depth: int) -> bool:
    return (
        isinstance(node, ast.Constant)
        and isinstance(node.value, str)
        and string_mentions_any(node.value, names, depth + 1)
    )


def _is_any_string(sf: SourceFile, node: ast.AST) -> bool:
    return (
        isinstance(node, ast.Constant)
        and isinstance(node.value, str)
        and string_mentions_any(node.value, sf.typing_names, 0)
    )


def _is_any_reference(sf: SourceFile, node: ast.AST) -> bool:
    if isinstance(node, ast.Name):
        return sf.typing_names.is_any(node.id)
    if isinstance(node, ast.Attribute):
        return is_typing_attribute(node, sf.typing_names)
    return _is_any_string(sf, node)


def references_typing_any(sf: SourceFile, diag: Diag) -> bool:
    """Whether gawain's GW020 sits on a reference to typing.Any. gawain reports at the start of the
    reference: a name that an import binds to Any, an attribute `typing.Any` through a typing module, or
    the opening quote of a string annotation whose content names Any."""
    column: int = diag.col - 1
    return any(
        _is_any_reference(sf, node)
        for node in sf.nodes_on(diag.line)
        if _reference_candidate(sf, node, column)
    )


def in_annotation(sf: SourceFile, diag: Diag) -> bool:
    position_of_diag: tuple[int, int] = (diag.line, diag.col - 1)
    return any(start <= position_of_diag < end for start, end in sf.annotation_spans)


def _target_column(sf: SourceFile, target: ast.expr) -> int | None:
    """The column where gawain reports an annotated target: its start, or the attribute name."""
    if isinstance(target, ast.Name):
        return sf.start_col(target)
    if isinstance(target, ast.Attribute):
        return sf.attr_col(target)
    return None


def _constant_target(node: ast.AnnAssign, sf: SourceFile) -> bool:
    """An UPPER_CASE name at module or class level, which gawain wants annotated as Final[T]."""
    target: ast.expr = node.target
    scope: ast.AST | None = sf.scope_of(node)
    return (
        isinstance(target, ast.Name)
        and is_constant(target.id)
        and isinstance(scope, (ast.Module, ast.ClassDef))
    )


def _constant_needs_final(node: ast.AnnAssign, sf: SourceFile) -> bool:
    """Whether gawain's GW010 holds for this annotated assignment: the annotation is neither Final[T] nor
    a TypeAlias."""
    return _constant_target(node, sf) and not (
        is_typed_final(node.annotation) or is_type_alias(node.annotation)
    )


def _final_rule_holds(diag: Diag, node: ast.AnnAssign, sf: SourceFile) -> bool:
    """Whether the Final rule that `diag` reports holds for `node`. A FINAL diagnostic is GW010 or GW011."""
    if diag.code == "GW011":
        return is_bare_final(node.annotation)
    return _constant_needs_final(node, sf)


def _annotated_at(sf: SourceFile, diag: Diag) -> ast.AnnAssign | None:
    """The annotated assignment whose target starts at the column of the diagnostic."""
    return next(
        (
            node
            for node in sf.nodes_on(diag.line)
            if isinstance(node, ast.AnnAssign) and _target_column(sf, node.target) == diag.col - 1
        ),
        None,
    )


def final_diag_explained(diag: Diag, sf: SourceFile) -> bool:
    """Whether the AnnAssign at the diagnostic's position is a Final form that gawain reports."""
    node: ast.AnnAssign | None = _annotated_at(sf, diag)
    return node is not None and _final_rule_holds(diag, node, sf)


def within_class(sf: SourceFile, node: ast.AST) -> bool:
    return any(isinstance(ancestor, ast.ClassDef) for ancestor in sf.ancestors(node))


def arg_nodes_on(sf: SourceFile, line: int, name: str) -> list[ast.arg]:
    return [node for node in sf.nodes_on(line) if isinstance(node, ast.arg) and node.arg == name]


def declares_type(assign: ast.Assign) -> bool:
    value: ast.expr = assign.value
    return (
        len(assign.targets) == 1
        and isinstance(assign.targets[0], ast.Name)
        and isinstance(value, ast.Call)
        and call_name(value.func) in TYPE_DECLARING_CALLS
    )


def rebinds(sf: SourceFile, assign: ast.Assign, name: str) -> bool:
    """Whether `name` is bound earlier in the scope of `assign`, in source order."""
    scope: ast.AST | None = sf.scope_of(assign)
    assert isinstance(scope, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef))
    targets: list[ast.expr] = [target for target in assign.targets if isinstance(target, ast.Name)]
    where: tuple[int, int] = position(targets[0])
    return any(bound == name and at < where for bound, at in sf.bindings_of(scope))


def class_body_owners(sf: SourceFile, diag: Diag) -> list[ast.ClassDef]:
    owners: list[ast.ClassDef] = []
    for assign in direct_assigns(sf, diag.line, name_of(diag)):
        owner: ast.AST | None = sf.parent(assign)
        if isinstance(owner, ast.ClassDef):
            owners.append(owner)
    return owners


def _is_class_init(node: ast.AST, sf: SourceFile) -> TypeIs[ast.FunctionDef | ast.AsyncFunctionDef]:
    return (
        isinstance(node, FUNCTION_NODES)
        and node.name == "__init__"
        and isinstance(sf.scope_of(node), ast.ClassDef)
    )


def _has_annotated_parameter(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> bool:
    return any(param.annotation is not None for param in judged_params(fn))


def init_with_annotated_parameter(sf: SourceFile, diag: Diag) -> bool:
    return any(
        _has_annotated_parameter(node) for node in sf.nodes_on(diag.line) if _is_class_init(node, sf)
    )


def _class_method_of(sf: SourceFile, arg: ast.arg) -> ast.FunctionDef | ast.AsyncFunctionDef | None:
    """The method that declares `arg`, when that method is in a class body."""
    method: ast.FunctionDef | ast.AsyncFunctionDef | None = arg_owner(sf, arg)
    if method is not None and isinstance(sf.scope_of(method), ast.ClassDef):
        return method
    return None


def _is_other_receiver(sf: SourceFile, arg: ast.arg, name: str) -> bool:
    method: ast.FunctionDef | ast.AsyncFunctionDef | None = _class_method_of(sf, arg)
    return method is not None and first_positional(method) is arg and name not in ("self", "cls")


def receiver_parameter_named_otherwise(sf: SourceFile, diag: Diag) -> bool:
    """A class-body method's receiver that is not named self or cls, which must-annotate reports."""
    name: str = name_of(diag)
    return any(_is_other_receiver(sf, arg, name) for arg in arg_nodes_on(sf, diag.line, name))


def _arg_at_column(sf: SourceFile, diag: Diag) -> ast.arg | None:
    """The first parameter that starts at the column of the diagnostic on its line."""
    return next(
        (
            node
            for node in sf.nodes_on(diag.line)
            if isinstance(node, ast.arg) and sf.start_col(node) == diag.col - 1
        ),
        None,
    )


def _is_static_receiver(sf: SourceFile, arg: ast.arg) -> bool:
    method: ast.FunctionDef | ast.AsyncFunctionDef | None = arg_owner(sf, arg)
    return method is not None and is_staticmethod(method) and raw_first_positional(method) is arg


def static_self_parameter(sf: SourceFile, diag: Diag) -> bool:
    """A staticmethod's first parameter named self or cls, which gawain judges and must-annotate skips."""
    arg: ast.arg | None = _arg_at_column(sf, diag)
    return arg is not None and arg.arg in ("self", "cls") and _is_static_receiver(sf, arg)


# --- Expectations --------------------------------------------------------------------------------


@dataclass(frozen=True)
class Ctx:
    sf: SourceFile
    project: Project


@dataclass(frozen=True)
class Expectation:
    name: str
    reason: str
    check: Callable[[Row, Ctx], bool]


def only_gawain_in(row: Row, families: tuple[str, ...]) -> bool:
    return row.only_gawain and row.family in families


def only_ma_in(row: Row, families: tuple[str, ...]) -> bool:
    return row.only_ma and row.family in families


def on_line_of_kind(sf: SourceFile, line: int, kinds: tuple[type[ast.AST], ...]) -> bool:
    return any(isinstance(node, kinds) for node in sf.nodes_on(line))


def pred_gawain_only_final(row: Row, ctx: Ctx) -> bool:
    return only_gawain_in(row, ("FINAL",)) and all(
        final_diag_explained(diag, ctx.sf) for diag in row.gawain
    )


def pred_gawain_only_any(row: Row, ctx: Ctx) -> bool:
    return only_gawain_in(row, ("ANY",)) and all(
        in_annotation(ctx.sf, diag) and references_typing_any(ctx.sf, diag) for diag in row.gawain
    )


def pred_gawain_only_suppression(row: Row, ctx: Ctx) -> bool:
    return only_gawain_in(row, ("SUPPRESSION",)) and all(
        gawain_malformed(ctx.sf, diag) or gawain_unused(ctx.sf, diag) for diag in row.gawain
    )


def pred_suppressed_by_gawain_comment(row: Row, ctx: Ctx) -> bool:
    entries: list[str] | None = directive_entries(ctx.sf, row.line)
    return (
        row.only_ma
        and entries is not None
        and bool(set(entries) & GAWAIN_CODES_BY_FAMILY[row.family])
    )


def pred_ma_loop_target(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("LOOP",)) and all(
        on_line_of_kind(ctx.sf, diag.line, LOOP_NODES) for diag in row.ma
    )


def _reassigned(sf: SourceFile, diag: Diag) -> bool:
    name: str = name_of(diag)
    return any(rebinds(sf, assign, name) for assign in direct_assigns(sf, diag.line, name))


def pred_reassigned_name(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("VAR", "ATTR")) and all(_reassigned(ctx.sf, diag) for diag in row.ma)


def pred_type_declaring_call(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("VAR", "ATTR")) and all(
        any(declares_type(assign) for assign in direct_assigns(ctx.sf, diag.line, name_of(diag)))
        for diag in row.ma
    )


def _enum_member(ctx: Ctx, diag: Diag) -> bool:
    return any(ctx.project.lineage(ctx.sf, cls).enum for cls in class_body_owners(ctx.sf, diag))


def pred_enum_member(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("ATTR",)) and all(_enum_member(ctx, diag) for diag in row.ma)


def _class_body_name(sf: SourceFile, diag: Diag) -> bool:
    name: str = name_of(diag)
    return (name == "_" or is_dunder(name)) and bool(class_body_owners(sf, diag))


def pred_class_body_dunder_or_underscore(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("ATTR",)) and all(_class_body_name(ctx.sf, diag) for diag in row.ma)


def pred_init_without_return(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("RETURN",)) and all(
        init_with_annotated_parameter(ctx.sf, diag) for diag in row.ma
    )


WriteAccepts: TypeAlias = Callable[[Write, Lineage], bool]


def _ma_attr_explained(ctx: Ctx, diag: Diag, accepts: WriteAccepts) -> bool:
    """Whether a must-annotate ATTR report on `diag` is a receiver write that `accepts` explains."""
    name: str = name_of(diag)
    if "." not in name:
        return False
    attr: str = name.split(".", 1)[1]
    writes: list[Write] = [w for w in receiver_writes_on(ctx.sf, diag.line) if w.attr == attr]
    return bool(writes) and all(
        accepts(write, ctx.project.lineage(ctx.sf, write.cls)) for write in writes
    )


def ma_attr_explained(row: Row, ctx: Ctx, accepts: WriteAccepts) -> bool:
    """Whether every must-annotate ATTR row on this line is a receiver write that `accepts` explains."""
    return only_ma_in(row, ("ATTR",)) and all(
        _ma_attr_explained(ctx, diag, accepts) for diag in row.ma
    )


def init_plain_write_declared_here(write: Write, _lin: Lineage) -> bool:
    return (
        write.kind == "plain"
        and write.method.name == "__init__"
        and write.attr in declared_names(write.cls)
    )


def init_plain_write_declared_above(write: Write, lin: Lineage) -> bool:
    return (
        write.kind == "plain"
        and write.method.name == "__init__"
        and write.attr not in declared_names(write.cls)
        and write.attr in lin.declared
    )


def init_plain_write_external(write: Write, lin: Lineage) -> bool:
    return write.kind == "plain" and write.method.name == "__init__" and lin.external


def pred_attr_declared_in_class(row: Row, ctx: Ctx) -> bool:
    return ma_attr_explained(row, ctx, init_plain_write_declared_here)


def pred_attr_declared_in_ancestor(row: Row, ctx: Ctx) -> bool:
    return ma_attr_explained(row, ctx, init_plain_write_declared_above)


def pred_attr_external_ancestor(row: Row, ctx: Ctx) -> bool:
    return ma_attr_explained(row, ctx, init_plain_write_external)


def pred_receiver_not_named_self(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("PARAM",)) and all(
        receiver_parameter_named_otherwise(ctx.sf, diag) for diag in row.ma
    )


def pred_static_self_parameter(row: Row, ctx: Ctx) -> bool:
    return only_gawain_in(row, ("PARAM",)) and all(
        static_self_parameter(ctx.sf, diag) for diag in row.gawain
    )


def _unpacked(sf: SourceFile, diag: Diag) -> bool:
    flags: list[bool] = [unpacked for name, unpacked in assigned_names(sf, diag.line) if name == name_of(diag)]
    return bool(flags) and all(flags)


def pred_unpacked_name(row: Row, ctx: Ctx) -> bool:
    return only_ma_in(row, ("VAR",)) and all(_unpacked(ctx.sf, diag) for diag in row.ma)


def _short_dunder_assigned(sf: SourceFile, diag: Diag) -> bool:
    name: str = name_of(diag)
    return is_short_dunder(name) and (name, False) in assigned_names(sf, diag.line)


def pred_dunder_four_underscores(row: Row, ctx: Ctx) -> bool:
    return only_gawain_in(row, ("VAR", "ATTR")) and all(
        _short_dunder_assigned(ctx.sf, diag) for diag in row.gawain
    )


def _in_class_method(ctx: Ctx, diag: Diag) -> bool:
    return any(
        within_class(ctx.sf, assign) for assign in direct_assigns(ctx.sf, diag.line, name_of(diag))
    )


def pred_ma_class_depth(row: Row, ctx: Ctx) -> bool:
    return only_gawain_in(row, ("VAR",)) and all(_in_class_method(ctx, diag) for diag in row.gawain)


def _gawain_attr_explained(ctx: Ctx, diag: Diag, accepts: WriteAccepts) -> bool:
    """Whether a gawain ATTR report on `diag` is a receiver write that `accepts` explains, with the
    attribute undeclared and the class free of external ancestors, which is what gawain reports."""
    writes: list[Write] = [write for write in receiver_writes_on(ctx.sf, diag.line) if write.col == diag.col - 1]
    return bool(writes) and all(
        accepts(write, ctx.project.lineage(ctx.sf, write.cls))
        and _undeclared(ctx.project.lineage(ctx.sf, write.cls), write)
        for write in writes
    )


def _undeclared(lin: Lineage, write: Write) -> bool:
    return not lin.external and write.attr not in lin.declared


def gawain_attr_explained(row: Row, ctx: Ctx, accepts: WriteAccepts) -> bool:
    return only_gawain_in(row, ("ATTR",)) and all(
        _gawain_attr_explained(ctx, diag, accepts) for diag in row.gawain
    )


def _is_annotated(write: Write, _lin: Lineage) -> bool:
    return write.kind == "annotated"


def _is_plain_outside_init(write: Write, _lin: Lineage) -> bool:
    return write.kind == "plain" and write.method.name != "__init__"


def _is_loop(write: Write, _lin: Lineage) -> bool:
    return write.kind == "loop"


def _is_unpacked(write: Write, _lin: Lineage) -> bool:
    return write.kind == "unpacked"


def pred_attr_annotated_write(row: Row, ctx: Ctx) -> bool:
    return gawain_attr_explained(row, ctx, _is_annotated)


def pred_attr_non_init_method(row: Row, ctx: Ctx) -> bool:
    return gawain_attr_explained(row, ctx, _is_plain_outside_init)


def pred_attr_loop_target(row: Row, ctx: Ctx) -> bool:
    return gawain_attr_explained(row, ctx, _is_loop)


def pred_attr_unpacked_write(row: Row, ctx: Ctx) -> bool:
    return gawain_attr_explained(row, ctx, _is_unpacked)


EXPECTED: Final[tuple[Expectation, ...]] = (
    Expectation(
        "gawain_only_final",
        "gawain only: must-annotate has no Final rule, so GW010 or GW011 on an annotated name is gawain's",
        pred_gawain_only_final,
    ),
    Expectation(
        "gawain_only_any",
        "gawain only: must-annotate has no Any rule, so GW020 on a reference to typing.Any in a type expression is gawain's",
        pred_gawain_only_any,
    ),
    Expectation(
        "gawain_only_suppression",
        "gawain only: GW900 reports a malformed `gawain: ignore` directive and GW901 an unused one, which must-annotate does not read",
        pred_gawain_only_suppression,
    ),
    Expectation(
        "suppressed_by_gawain_comment",
        "a `gawain: ignore[...]` directive on this line silences the gawain code of this family",
        pred_suppressed_by_gawain_comment,
    ),
    Expectation(
        "ma_loop_target",
        "for and with targets are exempt in gawain; must-annotate reports them only under the opt-in TF005",
        pred_ma_loop_target,
    ),
    Expectation(
        "reassigned_name",
        "annotation is required only at the first binding in a scope, so a later assignment is free",
        pred_reassigned_name,
    ),
    Expectation(
        "type_declaring_call",
        "a TypeVar, ParamSpec, TypeVarTuple, NewType, NamedTuple or TypedDict value declares its type, so gawain exempts it",
        pred_type_declaring_call,
    ),
    Expectation(
        "enum_member",
        "a member of an Enum class is a member, not an attribute, so gawain exempts it",
        pred_enum_member,
    ),
    Expectation(
        "class_body_dunder_or_underscore",
        "gawain never reports a dunder or `_` in a class body; must-annotate does",
        pred_class_body_dunder_or_underscore,
    ),
    Expectation(
        "init_without_return",
        "__init__ may omit `-> None` when another judged parameter is annotated",
        pred_init_without_return,
    ),
    Expectation(
        "attr_declared_in_class",
        "self.x is declared in the class body, which satisfies gawain's attribute rule",
        pred_attr_declared_in_class,
    ),
    Expectation(
        "attr_declared_in_ancestor",
        "self.x is declared by a project ancestor, which satisfies gawain's attribute rule",
        pred_attr_declared_in_ancestor,
    ),
    Expectation(
        "attr_external_ancestor",
        "an external ancestor releases gawain's attribute rule; must-annotate checks __init__ anyway",
        pred_attr_external_ancestor,
    ),
    Expectation(
        "receiver_not_named_self",
        "gawain takes the first positional parameter as the receiver whatever its name; must-annotate exempts only self and cls",
        pred_receiver_not_named_self,
    ),
    Expectation(
        "static_self_parameter",
        "a staticmethod's first parameter is not exempt in gawain; must-annotate exempts self and cls by name",
        pred_static_self_parameter,
    ),
    Expectation(
        "unpacked_name",
        "gawain exempts names inside an unpacking target (glossary: exempt target); must-annotate reports them",
        pred_unpacked_name,
    ),
    Expectation(
        "dunder_four_underscores",
        "gawain needs more than four characters for a dunder (fixture variables.py); must-annotate skips `____`",
        pred_dunder_four_underscores,
    ),
    Expectation(
        "ma_class_depth",
        "must-annotate skips TF001 in any function nested under a class, not only in the class body (its bug)",
        pred_ma_class_depth,
    ),
    Expectation(
        "attr_annotated_write",
        "an annotated self.x: T in a method does not declare x; must-annotate does not read annotated assignments",
        pred_attr_annotated_write,
    ),
    Expectation(
        "attr_non_init_method",
        "must-annotate checks self attributes only in __init__; gawain judges receiver writes in every method",
        pred_attr_non_init_method,
    ),
    Expectation(
        "attr_loop_target",
        "must-annotate does not check for or with targets on a receiver; gawain does (fixture classes.py)",
        pred_attr_loop_target,
    ),
    Expectation(
        "attr_unpacked_write",
        "must-annotate does not check unpacked receiver attributes; gawain does (fixture classes.py:42)",
        pred_attr_unpacked_write,
    ),
)


# --- Comparison ----------------------------------------------------------------------------------


def _is_unit(entry: Path) -> bool:
    return entry.is_dir() or (entry.is_file() and entry.suffix == ".py")


def units_of(root: Path) -> list[Path]:
    """Each top-level .py file and each subdirectory of `root`, sorted, as tests/fixtures.rs runs them."""
    return [entry for entry in sorted(root.iterdir()) if _is_unit(entry)]


def source_paths(unit: Path) -> list[Path]:
    return [unit] if unit.is_file() else sorted(unit.rglob("*.py"))


def _row_for(rows: dict[tuple[str, int, str], Row], diag: Diag) -> Row:
    key: tuple[str, int, str] = (diag.name, diag.line, diag.family)
    if key not in rows:
        rows[key] = Row(diag.name, diag.line, diag.family)
    return rows[key]


def group_rows(gawain: list[Diag], ma: list[Diag]) -> list[Row]:
    rows: dict[tuple[str, int, str], Row] = {}
    for diag in gawain:
        _row_for(rows, diag).gawain.append(diag)
    for diag in ma:
        _row_for(rows, diag).ma.append(diag)
    return list(rows.values())


def _directive_entries_in(comment: str) -> list[str] | None:
    found: re.Match[str] | None = DIRECTIVE_RE.search(comment)
    if found is None:
        return None
    listed: re.Match[str] | None = DIRECTIVE_CODES_RE.match(comment, found.end())
    if listed is None:
        return []
    return [entry.strip() for entry in listed.group(1).split(",") if entry.strip()]


def _classify_one_sided(row: Row, ctx: Ctx) -> None:
    expectation: Expectation | None = _explaining(row, ctx)
    if expectation is None:
        row.status = "unexplained"
        row.reason = "no EXPECTED predicate explains this"
        return
    row.status = "expected"
    row.reason = expectation.reason
    row.explained_by = expectation.name


def _explaining(row: Row, ctx: Ctx) -> Expectation | None:
    """The first EXPECTED predicate that explains the row, or None."""
    return next((e for e in EXPECTED if e.check(row, ctx)), None)


def classify(row: Row, ctx: Ctx) -> None:
    if row.gawain and row.ma:
        row.status = "agree"
    else:
        _classify_one_sided(row, ctx)


def compare_unit(unit: Path, root: Path, config_dir: Path) -> list[Row]:
    gawain_diags: list[Diag] = run_gawain(unit, root)
    ma_diags: list[Diag] = run_must_annotate(unit, root, config_dir)
    sources: list[SourceFile] = [
        load_source(path, relative_name(path, root)) for path in source_paths(unit)
    ]
    project: Project = build_project(sources)
    rows: list[Row] = group_rows(gawain_diags, ma_diags)
    for row in rows:
        sf: SourceFile | None = project.by_name.get(row.name)
        if sf is None:
            raise CompareError(f"{row.name} has a diagnostic but is not part of {unit.name}")
        classify(row, Ctx(sf, project))
    return rows


def codes_text(diags: list[Diag]) -> str:
    codes: list[str] = sorted({diag.code for diag in diags})
    return ",".join(codes) if codes else "-"


def render_table(rows: list[Row]) -> list[str]:
    """The table: file, line, gawain, must-annotate and status in padded columns, then the reason."""
    header: list[str] = ["file", "line", "gawain", "must-annotate", "status", "reason"]
    body: list[list[str]] = [
        [row.name, str(row.line), codes_text(row.gawain), codes_text(row.ma), row.status, row.reason]
        for row in rows
    ]
    widths: list[int] = [max(len(cells[index]) for cells in [header, *body]) for index in range(5)]
    lines: list[str] = []
    for cells in [header, *body]:
        head: list[str] = [cell.ljust(width) for cell, width in zip(cells[:5], widths)]
        lines.append("  ".join([*head, cells[5]]).rstrip())
    return lines


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser: argparse.ArgumentParser = argparse.ArgumentParser(
        description="Compare gawain with must-annotate on the fixtures."
    )
    default_root: str = str(REPO_ROOT / "tests" / "fixtures")
    parser.add_argument(
        "root",
        nargs="?",
        default=default_root,
        help="fixtures root; each top-level .py file and each subdirectory is one unit",
    )
    parser.add_argument(
        "--hide-agree", action="store_true", help="leave the agreeing rows out of the table"
    )
    return parser.parse_args(argv)


def check_root(root: Path) -> list[Row]:
    """Every row of every unit under `root`, sorted by file, line and family."""
    rows: list[Row] = []
    with tempfile.TemporaryDirectory(prefix="must-annotate-config-") as config:
        Path(config, "pyproject.toml").write_text(MUST_ANNOTATE_CONFIG, encoding="utf-8")
        for unit in units_of(root):
            rows.extend(compare_unit(unit, root, Path(config)))
    rows.sort(key=lambda row: (row.name, row.line, row.family))
    return rows


def _print_table(rows: list[Row], hide_agree: bool) -> None:
    shown: list[Row] = [row for row in rows if not (hide_agree and row.status == "agree")]
    print("\n".join(render_table(shown)))


def _print_summary(rows: list[Row]) -> None:
    counts: Counter[str] = Counter(row.status for row in rows)
    print(
        f"{counts['agree']} agree · {counts['expected']} expected · {counts['unexplained']} unexplained"
    )


def _print_predicates(rows: list[Row]) -> None:
    by_predicate: Counter[str] = Counter(row.explained_by for row in rows if row.status == "expected")
    print("expected rows by predicate:")
    for expectation in EXPECTED:
        print(f"  {by_predicate[expectation.name]:>4}  {expectation.name}: {expectation.reason}")


def print_report(rows: list[Row], hide_agree: bool) -> int:
    """Prints the table, the summary and the count of rows per predicate. Returns 1 when a row is
    unexplained, and 0 otherwise."""
    _print_table(rows, hide_agree)
    print()
    _print_summary(rows)
    _print_predicates(rows)
    return 1 if any(row.status == "unexplained" for row in rows) else 0


def main(argv: list[str] | None = None) -> int:
    """The exit status: 0 when every row is agreed or explained, 1 when a row is unexplained, and 2
    when a tool cannot run or prints something unexpected."""
    args: argparse.Namespace = parse_args(argv)
    try:
        if not GAWAIN.is_file():
            raise CompareError(f"gawain binary not found at {GAWAIN}; run `cargo build` first")
        root: Path = Path(args.root).resolve()
        if not root.is_dir():
            raise CompareError(f"{root} is not a directory")
        rows: list[Row] = check_root(root)
    except CompareError as error:
        print(f"compare: {error}", file=sys.stderr)
        return 2
    return print_report(rows, args.hide_agree)


if __name__ == "__main__":
    sys.exit(main())
