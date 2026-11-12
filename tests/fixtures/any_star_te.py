# A bare Any after from typing_extensions import * is typing.Any, as typing_extensions re-exports it (GW020).
from typing_extensions import *

x: Any = 1  # expect: GW020
y: list[Any] = []  # expect: GW020
