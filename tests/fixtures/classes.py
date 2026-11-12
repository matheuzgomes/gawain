# Class bodies and attributes (GW004). A plain assignment in a class body needs an annotation. An
# attribute bound through the first positional parameter of a method (or a classmethod's cls) must be
# declared in its class or in an ancestor in the project. A class with an external ancestor skips the
# attribute rule but not the class-body rule. An Enum class is exempt from the class-body rule.
import enum
from enum import Enum
from typing import Generic, TypeVar

T = TypeVar("T")


class Plain:
    x = 1  # expect: GW004
    COLOR = "red"  # expect: GW004
    MAX_SIZE = 3  # expect: GW004
    _ = 4
    __slots__ = ("a",)
    __hash__ = None
    a, b = 1, 2
    Item = TypeVar("Item")
    counter += 1


class Annotated:
    x: int = 1
    x = 2

    def size(self) -> int:
        return 1

    size = 2


class Named:
    name: str
    a: int

    def __init__(self, n: str) -> None:
        self.name = n
        self.email = ""  # expect: GW004
        self.age: int = 0  # expect: GW004
        self.a, self.b = 1, 2  # expect: GW004
        for self.i in range(3):  # expect: GW004
            pass
        with open("log.txt") as self.handle:  # expect: GW004
            pass


class Tally:
    def __init__(self, n: int) -> None:
        self.n += 1
        self.n = n  # expect: GW004


class Registry:
    @classmethod
    def build(cls) -> None:
        cls.count = 0  # expect: GW004


class Helper:
    @staticmethod
    def stamp(p: object) -> None:
        p.x = 1


class Outer:
    def run(self, item: int) -> None:
        def inner(value: int) -> None:
            self.z = value

        inner(item)


class Renamed:
    def bump(this, step: int) -> None:
        this.count = step  # expect: GW004
        this.inner.value = 1
        this.items[0] = 1

    def other(self, peer: "Renamed") -> None:
        peer.count = 1


class Gather:
    def collect(*items: int) -> None:
        items.last = 1


class Color(Enum):
    RED = 1
    GREEN = 2


class Mode(enum.IntEnum):
    A = 1


class Palette(Enum):
    pass


class Shade(Palette):
    LIGHT = 1


class MyErr(Exception):
    LEVEL = 3  # expect: GW004

    def __init__(self, code: int) -> None:
        self.code = code


class Box(Generic[T]):
    def __init__(self, v: int) -> None:
        self.v = v  # expect: GW004


class Base(Generic[T]):
    v: T


class Typed(Base[int]):
    def __init__(self) -> None:
        self.v = 1


class Child(Base):
    def __init__(self) -> None:
        self.v = 2
        self.extra = 3  # expect: GW004


class Plainer(object):
    def __init__(self) -> None:
        self.x = 1  # expect: GW004


class Root:
    shared: int


class Left(Root):
    pass


class Right(Root):
    pass


class Both(Left, Right):
    def __init__(self) -> None:
        self.shared = 1
        self.extra = 2  # expect: GW004


class Ping(Pong):
    def __init__(self) -> None:
        self.x = 1  # expect: GW004


class Pong(Ping):
    pass


def make_codec() -> type:
    class Codec(dict):
        def __init__(self) -> None:
            self.mode = 1

    return Codec


def make_palette() -> type:
    class Tone(Enum):
        DARK = 1

    return Tone


def make_shadow() -> type:
    class Root(dict):
        def __init__(self) -> None:
            self.anything = 1  # ok: dict is external, and this Root is not the module-level Root

    return Root


def make_child() -> type:
    class Inner(Root):
        LIMIT = 1  # expect: GW004

        def __init__(self) -> None:
            self.shared = 1
            self.bogus = 2  # expect: GW004

    return Inner
