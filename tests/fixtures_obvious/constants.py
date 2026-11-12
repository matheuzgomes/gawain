# Constants (UPPER_CASE at module or class level) need Final even when their value is obvious: GW001 or GW004.
# A bare Final with an obvious value is allowed. A bare Final with any other value, or with no value, is GW011.
from typing import Final


MAX: Final = 3
LIMIT: Final[int] = 4
T2: Final = load()  # expect: GW011
NOTHING: Final = None  # expect: GW011
MAX_TWO = 3  # expect: GW001
TIMEOUT = 30  # expect: GW001
_PRIVATE = 1  # expect: GW001
other: Final  # expect: GW011
name: Final = "x"
size: Final = 3


class Settings:
    LIMIT_CLASS = 5  # expect: GW004
    COLOR: Final = "red"
    RATE: Final = load()  # expect: GW011


def compute() -> int:
    LOCAL_LIMIT = 4
    LOCAL_ALIAS: Final = 5
    LOCAL_NAME: Final = load()  # expect: GW011
    return LOCAL_LIMIT + LOCAL_ALIAS
