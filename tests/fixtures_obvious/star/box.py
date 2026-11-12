from typing import Generic, TypeVar

T = TypeVar("T")


class Box(Generic[T]):
    pass


class _Hidden(Generic[T]):
    pass
