# A class named Any is not typing.Any, and neither is the Any of another module. With no typing import,
# Any in an annotation is an ordinary name, so nothing here is reported.
import mylib


class Any:
    """A class of this module's own."""


def make(a: Any) -> Any:
    return a


value: Any = Any()
other: mylib.Any = 1
