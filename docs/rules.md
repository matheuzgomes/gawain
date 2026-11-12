# Rules

The rules of the presence phase, the two modes, the exemptions and suppression. The README has a short version.

## Rules

| Code | What it reports |
| --- | --- |
| GW000 | A file that cannot be checked: it has a syntax error, it is not valid UTF-8, or it cannot be read. A directory that cannot be listed while walking, or any other path that the walk cannot read, gives one GW000 at its path. No other rule runs on a file with GW000, and GW000 cannot be suppressed. |
| GW001 | A variable in a module or function scope is assigned with no annotation. |
| GW002 | A function parameter has no annotation. |
| GW003 | A function has no return annotation. |
| GW004 | A class-body attribute has no annotation, or `self.x` is set and neither the class nor an ancestor declares `x`. |
| GW010 | A constant (an UPPER_CASE name at module or class level) is not annotated `Final[T]`. |
| GW011 | A bare `Final` with no type argument. |
| GW020 | `Any` in a type expression. |
| GW900 | A malformed `gawain: ignore` comment. It cannot be suppressed. |
| GW901 | A `gawain: ignore` code that silences nothing. It cannot be suppressed. |

The type checker's own diagnostics are printed with its name and its rule, such as `ty[invalid-assignment]`
or `pyrefly[bad-assignment]`, and they have no GW code.

## Modes

The mode decides whether the first assignment of a name needs an annotation when its value is obvious. It is
`obvious` by default, and `strict` makes every plain assignment need one, as gawain did before modes existed:

```toml
[tool.gawain]
mode = "strict"
```

In obvious mode the rules below apply to GW001 (module and function scope), to GW004 (a class body), and to
GW011 (a bare `Final`). Every other rule is the same in both modes.

| Obvious: no annotation needed | Annotation required |
| --- | --- |
| `count = 0`, `name = "ada"`, `ratio = -1.5`, `flag = True` | `value = None`, `value = ...`, `value = load()` |
| `numbers = [1, 2, 3]`, `pair = (1, "a")`, `table = {"a": [1]}` | `mixed = [1, 2.5]`, `empty = []`, `nested = [[1], ["a"]]` |
| `user = User("ada")`, `model = models.User()`, `words = Counter(lines)` | `counter = Counter()`, `url = URL("x")`, `moment = datetime(2020, 1, 1)` |
| `MAX: Final = 3`, `NAME: Final = "ada"` | `MAX = 3` (a constant needs `Final`), `LIMIT: Final = load()` |
| class body: `count = 0`, with no method that sets `self.count` | class body: `count = 0`, with `self.count = 1` in a method |
| `hits = 0`, then `hits += 1` | `count = 0`, then `count = count + 1` |

The rules:

- **Literal:** a number, a string (implicit concatenation and f-strings included), bytes, or a bool, with an
  optional `-` or `+` in front of a number. `None`, `...` and t-strings are not literals.
- **Container of literals:** a non-empty list, set, tuple or dict of literals, containers included. The items of
  a list, a set, or a dict (its keys and its values) must share one shape: `[1, 2]` is obvious and `[1, 2.5]`
  is not. A container inside a container follows the same rule, so `[[1], [2]]` is obvious and `[[1], ["a"]]` is
  not. A tuple may mix kinds: `(1, "a")` is obvious. An empty container anywhere, a `*` or `**` item, or a call
  inside a container makes the value not obvious.
- **Constructor:** a call whose callee is a name in CapWords, or an attribute whose last part is one, such as
  `User("ada")` or `models.User()`. A name is in CapWords when it starts with an uppercase letter after any
  leading underscores and has a lowercase letter, so `URL` and `MAX` are not, and neither is `datetime(...)`.
  A call with arguments is obvious. A call with no arguments is not obvious when the class is a known generic
  (`Counter`, `OrderedDict`, `Queue`, `WeakSet`, `asyncio.Future` and the other standard generic classes) or a
  generic class of the project. A project class is generic when it has PEP 695 type parameters, a `Generic[...]`
  or `Protocol[...]` base, or a base subscripted with a type variable that its module defines or imports from a
  project module, such as `Box[T]` or `Box[tv.T]`. A base written without a subscript passes on its own type
  parameters, so `class Sub(Box)` is generic when `Box` is. A
  base subscripted with concrete types, such as `Box[int]`, gives its subclass none, so `IntBox()` is obvious for
  `class IntBox(Box[int])`. So `Counter(lines)` is obvious and `Counter()` is not, and `Box()` is not when
  `class Box(Generic[T])` is in the project.
- **Star import:** in obvious mode, a name that a module does not bind is looked up in the project modules that it
  star-imports, last import first. So after `from box import *`, `Box()` is judged by the `Box` of `box`. A star
  import of a module outside the analysed files is not followed, and a name that starts with an underscore is
  never star-imported. Strict mode does not follow star imports, so its output stays as it was.

A first assignment that is obvious passes only if the name is never bound again in the same scope. An augmented
assignment (`+=`) does not count as a rebinding. Anything else does: a later assignment, an annotation, a `for`,
`with`, `except`, `match` or walrus target, an import, a `def` or `class`, and a nested function that binds the
name through `nonlocal` or `global`. The first assignment is then reported as GW001, or GW004 in a class body,
with the line of the first rebinding:

```text
ledger.py:1:1  GW001 variable 'count' has no type annotation (it is reassigned on line 2)
```

Constants, UPPER_CASE names at module or class level, are never exempt from GW001 or GW004, because they need
`Final[T]`. A bare `Final` is exempt in GW011 when its value is obvious, in any scope: `MAX: Final = 3` passes,
and `LIMIT: Final = load()` is GW011.

An obvious assignment in a class body passes when no other binding of the name is in the body, and no method
assigns it through its receiver, `self.count = ...` or `cls.count = ...`. Such an assignment declares its
attribute for the classes of the project that inherit from it, so a subclass may assign `self.count` in its own
methods. Any other plain assignment in a class body is GW004 as before.

When a method does assign an obvious class-body name, a constant included, through its receiver, the one GW004
is on the class-body line, with the line of the assignment, and the method's plain assignment is not reported
again. An annotated assignment through the receiver, such as `self.count: int = 1`, is still GW004 on its own
line.

## Exemptions

Presence does not report:

- `_` and dunder names such as `__version__`.
- `for`, `with`, `except`, `match` and walrus targets, comprehension targets, and names inside an unpacking
  assignment.
- The target of an assignment whose value declares a type, such as `T = TypeVar("T")`.
- Plain assignments in the body of an `Enum` class (`IntEnum`, `StrEnum`, `Flag`, `IntFlag` and `ReprEnum`
  too), because they are members.
- The `self` or `cls` parameter of a method. A `@staticmethod` has no receiver.
- In obvious mode, a first assignment of an obvious value, which [Modes](#modes) describes.
- The return annotation of an `__init__` in a class body, once one of its other parameters is annotated.
- `TypeAlias` annotations, for GW010.
- Attributes of a class with an external base, such as a class from another library, a builtin, or a name
  gawain cannot resolve. `object`, `Generic` and `Protocol` do not count as external.

For GW020, only `Any` that an import binds to `typing.Any` counts. A class that happens to be called `Any` is
not reported. The arguments of `Literal` and the metadata of `Annotated` are not checked.

## Suppression

A comment silences the listed codes on its own line and nothing else:

```python
value = load()  # gawain: ignore[GW001]
def add(a, b): ...  # gawain: ignore[GW002, GW003]
```

A code that silences nothing is reported as GW901. A comment with no valid code is reported as GW900.
