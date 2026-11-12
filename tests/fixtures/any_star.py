# A bare Any after from typing import * is typing.Any (GW020).
from typing import *

x: Any = 1  # expect: GW020
y: list[Any] = []  # expect: GW020
