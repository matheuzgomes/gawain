# Variables without an annotation at their first binding (GW001). Every function is fully annotated,
# so the only diagnostics here are GW001 ones and the GW004 class attributes. A line with an expect
# marker reports exactly those codes. A name that is bound again later in the same scope is silent.
import types
import typing
from typing import NamedTuple, NewType, ParamSpec, TypeAlias, TypedDict, TypeVar, TypeVarTuple


# Module scope: the first assignment reports, later ones are silent.
limit = 10  # expect: GW001
limit = 20
limit += 1
MAX_RETRIES = 3  # expect: GW001

if limit > 5:
    mode = "wide"  # expect: GW001
else:
    mode = "narrow"

# A bare declaration binds the name, so the branches below are silent.
status: str
if limit > 5:
    status = "wide"
else:
    status = "narrow"

# An annotated assignment binds the name, so the reassignment is silent.
count: int = 0
count = count + 1

# A chained assignment reports each target.
first_slot = second_slot = 0  # expect: GW001 GW001

# Unpacking, including nested and starred targets, binds without reporting.
left, right = (1, 2)
[head, *tail] = [3, 4, 5]
first, (second, third) = 6, (7, 8)

# For, with, except and walrus targets bind without reporting.
for index, item in enumerate(["a", "b"]):
    print(index, item)
index = 9

with open("notes.txt") as handle:
    contents = handle.read()  # expect: GW001
handle = None

try:
    value = int("4")  # expect: GW001
except ValueError as failure:
    print(failure)
failure = None

if (found := limit * 2) > 10:
    print(found)
found = 0
print(zed := 1)
zed = 2

# A comprehension target is local to the comprehension, so n is first bound by the assignment below.
squares = [n * n for n in range(3)]  # expect: GW001
n = 0  # expect: GW001
# A walrus inside a comprehension binds in the enclosing scope, so the later assignment is silent.
doubled = [y for x in range(4) if (y := x * 2) > 2]  # expect: GW001
y = 0

# A walrus inside a lambda binds in the lambda's own scope, so the module name is a first binding.
probe = lambda: (inner_mark := 1)  # expect: GW001
inner_mark = 2  # expect: GW001
scale = lambda factor: factor  # expect: GW001
factor = 1  # expect: GW001
# A lambda parameter default is evaluated in the enclosing scope, as a def default is.
gain = lambda step=(scale_base := 2): step  # expect: GW001
scale_base = 3

# Names that are never reported.
_ = limit
_ = 2
__all__ = ["limit"]
__version__ = "1.0"

# Four underscores begin and end with two underscores, but a dunder needs more than four characters.
____ = 0  # expect: GW001
# Ending in two underscores is not enough, because a dunder also begins with them.
trailing__ = 0  # expect: GW001

# Attribute and subscript targets are not names, so this rule ignores them.
settings: types.SimpleNamespace = types.SimpleNamespace()
settings.debug = True
registry: dict[str, int] = {}
registry["key"] = 1
helper_state.flag = True

# An augmented assignment neither reports nor binds, so the plain assignment is the first binding.
hits += 1
hits = 0  # expect: GW001

# Calls that declare a type are exempt, bare and qualified.
Item = TypeVar("Item")
Call = ParamSpec("Call")
Shape = TypeVarTuple("Shape")
Ident = NewType("Ident", int)
Pair = NamedTuple("Pair", [("left", int), ("right", int)])
Record = TypedDict("Record", {"name": str})
Tag = typing.TypeVar("Tag")
Slot = typing.ParamSpec("Slot")
Gap = typing.TypeVarTuple("Gap")
Token = typing.NewType("Token", str)
Point = typing.NamedTuple("Point", [("x", int), ("y", int)])
Entry = typing.TypedDict("Entry", {"key": str})
Other = make_factory("Other")  # expect: GW001
Cast = typing.cast(int, 1)  # expect: GW001

# Old-style alias reports, the annotated alias does not.
OldAlias = int  # expect: GW001
NewAlias: TypeAlias = int

# A type statement binds its name.
type Label = str
Label = bytes

# Imports bind their names, so reassigning an imported name is silent.
import json
json = None
import collections.abc
collections = None
from os import sep
sep = "-"
from pathlib import Path as P
P = None

# A def or class binds its name in the enclosing scope.
def wrap(function: object) -> object:
    return function


def handler() -> None:
    return None


handler = wrap(handler)


class Gauge:
    pass


Gauge = wrap(Gauge)


# Class bodies are not judged here: a class attribute is GW004. The method body is a function scope.
class Settings:
    limit = 10  # expect: GW004

    def describe(self) -> str:
        y = 2  # expect: GW001
        return str(y)


class Counter:
    def __init__(self, start: int) -> None:
        self.value = start  # expect: GW004


# Function scopes: parameters are bound on entry, and each scope starts with nothing bound.
def scale_total(factor: int) -> int:
    factor = factor * 2
    return factor


def local_names() -> int:
    partial = 1  # expect: GW001
    partial = partial + 1
    return partial


def outer_total() -> int:
    partial = 1  # expect: GW001

    def inner_total() -> int:
        partial = 2  # expect: GW001
        return partial

    return partial + inner_total()


def pick_label(wide: bool) -> str:
    label: str
    if wide:
        label = "wide"
    else:
        label = "narrow"
    return label


def tagged() -> None:
    __marker__ = 1


# A module name assigned in a function without global is a new local.
counter: int = 0


def bump_counter() -> None:
    counter = counter + 1  # expect: GW001


def reset_counter() -> None:
    global counter
    counter = 0


def make_tally() -> int:
    tally: int = 0

    def add_one() -> None:
        nonlocal tally
        tally = tally + 1

    add_one()
    return tally


# A default value is evaluated in the enclosing scope, so its walrus binds the module name.
def pick(value: int = (fallback := 3)) -> int:
    return value


fallback = 4

# A global statement outside a function binds nothing.
global beacon
beacon = 1  # expect: GW001


async def drain(lines: list[str]) -> int:
    size = 0  # expect: GW001
    async for line in lines_stream(lines):
        size = size + len(line)
    async with open_gate() as gate:
        size = size + 1
    line = ""
    gate = None
    return size


def route(command: object) -> str:
    match command:
        case [verb, *args]:
            args = list(args)
            return str(verb) + str(args)
        case {"name": name, **extra}:
            extra = dict(extra)
            return str(name) + str(extra)
        case str() as text:
            text = text.strip()
            return text
        case _:
            return "none"
