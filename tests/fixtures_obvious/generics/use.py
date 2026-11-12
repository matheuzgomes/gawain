from typing import TypeVar

from box import Box

T = TypeVar("T")


class Sub(Box[int]):
    pass


class Bare(Box):
    pass


class Wrap(Box[T]):
    pass


class Deeper(Sub):
    pass


empty = Box()  # expect: GW001
filled = Box(1)
inherited = Sub()
bare = Bare()  # expect: GW001
wrapped = Wrap()  # expect: GW001
deeper = Deeper()
