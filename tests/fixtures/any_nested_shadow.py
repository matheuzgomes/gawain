# A module-level import decides what a module-level Any is. Here it binds mylib's Any, so the annotation
# is not reported, even though a function below imports typing.Any under the same name.
from mylib import Any


def helper() -> None:
    from typing import Any


x: Any = 1
