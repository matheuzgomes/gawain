# Constants: an UPPER_CASE name at module or class level must be annotated as Final[T] (GW010). A
# bare Final is reported wherever it annotates a name or an attribute (GW011). Every other name is
# fully annotated, so the only diagnostics are the marked ones. Dunder names are never constants.
import typing
import typing as t
from typing import Final, TypeAlias


MAX: Final[int] = 3
TIMEOUT: Final = 30  # expect: GW011
BASE_URL: str = "x"  # expect: GW010
PORT = 8080  # expect: GW001
TYPING_FORM: typing.Final[int] = 1
T_FORM: t.Final[str] = "x"
QUOTED_FORM: "Final[int]" = 2
QUOTED_PLAIN: "str" = "x"  # expect: GW010
QUOTED_BARE: "Final" = 3  # expect: GW011
QUOTED_BROKEN: "Final[" = 4  # expect: GW010
BARE_TYPING: typing.Final = 5  # expect: GW011
BARE_T: t.Final = 6  # expect: GW011
_PRIVATE: int = 1  # expect: GW010
MAX_2: int = 1  # expect: GW010
HTTP_OK: Final[int] = 200
x: Final = 1  # expect: GW011
JSON: TypeAlias = dict[str, object]
ALIAS_QUOTED: "TypeAlias" = int
ALIAS_ATTRIBUTE: typing.TypeAlias = int
TABLE: dict[str, int] = {}  # expect: GW010
__version__: str = "1"
__BUILD__: int = 7
Ratio: float = 1.0
_2FA: int = 9
RATE: float = 1.0  # expect: GW010
size: Final = 3  # expect: GW011
LIMIT_MODULE: int  # expect: GW010


class Settings:
    LIMIT: Final[int] = 5
    COLOR = "red"  # expect: GW004
    CLASS_RATE: float = 1.0  # expect: GW010
    CLASS_TIMEOUT: Final = 9  # expect: GW011
    CLASS_ALIAS: TypeAlias = int
    lowercase_value: Final = 2  # expect: GW011

    def __init__(self) -> None:
        self.v: Final = 1  # expect: GW011 GW004
        self.w: int = 2  # expect: GW004
        self.LIMIT_SELF: int = 3  # expect: GW004


def compute() -> int:
    LOCAL: int = 5
    LOCAL2: Final = 5  # expect: GW011
    return LOCAL + LOCAL2
