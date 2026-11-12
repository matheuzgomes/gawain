# A module that star-imports typing but also binds Any itself. The explicit import decides what Any is,
# so this Any is not typing.Any and nothing is reported.
from typing import *
from mylib import Any

x: Any = 1
