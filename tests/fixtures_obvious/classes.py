# Class-body assignments (GW004). An obvious plain assignment in a class body is exempt when no other
# binding of the name is in the body and no method assigns it through its receiver (self or cls). Any other
# plain assignment is GW004. An exempt assignment declares its attribute for the classes that inherit it.
# When a method rebinds an obvious class-body name through its receiver, the class-body line gets the one GW004,
# and the method's plain assignment is not reported again. An annotated assignment through the receiver is GW004.
from enum import Enum
from typing import Final


class Counters:
    count = 0
    limit = 10

    def describe(self) -> int:
        return self.count


class Tally:
    count = 0  # expect: GW004

    def bump(self) -> None:
        self.count = 1


class Registry:
    total = 0  # expect: GW004

    @classmethod
    def reset(cls) -> None:
        cls.total = 0


class Ticker:
    tick = 0  # expect: GW004

    def advance(self) -> None:
        self.tick: int = 1  # expect: GW004


class Source:
    origin = load()  # expect: GW004

    def reset(self) -> None:
        self.origin = None  # expect: GW004


class Stray:
    def set_it(self) -> None:
        self.never = 1  # expect: GW004


class Limiter:
    LIMIT = 5  # expect: GW004

    def raise_limit(self) -> None:
        self.LIMIT = 6


class Helper:
    def touch(self, helper: object) -> None:
        helper.count = 1


class Stepper:
    step = 1  # expect: GW004
    step = 2


class Sized:
    size = 0  # expect: GW004
    size: int = 1


class Accumulator:
    total = 0
    total += 1


class Limits:
    LIMIT = 5  # expect: GW004
    FIXED: Final = 6
    lowercase: Final = 7


class Loader:
    source = load()  # expect: GW004


class Holder:
    level: Final = 1

    def __init__(self) -> None:
        self.inner: Final = 2  # expect: GW004


class Color(Enum):
    RED = 1
