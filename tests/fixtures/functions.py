# Parameters and return annotations. A line with an expect marker reports exactly those codes.
import builtins


def plain(  # expect: GW003
    a,  # expect: GW002
    b,  # expect: GW002
):
    return a


def typed(
    a: int,
    b: str = "x",
) -> bool:
    return a > 0


def defaulted(
    count=3,  # expect: GW002
) -> None:
    pass


def varargs(
    *args,  # expect: GW002
    **kwargs,  # expect: GW002
) -> None:
    pass


def typed_varargs(
    *args: int,
    **kwargs: str,
) -> None:
    pass


def keyword_only(
    a: int,
    *,
    b,  # expect: GW002
) -> None:
    pass


def positional_only(
    a,  # expect: GW002
    /,
    b: int,
) -> None:
    pass


async def fetch(  # expect: GW003
    url,  # expect: GW002
):
    return url


def outer(
    value: int,
) -> int:
    def inner(  # expect: GW003
        item,  # expect: GW002
    ):
        return item

    return inner(value)


def make_pair() -> None:
    class Inner:
        pass

    def helper_after_class(  # expect: GW003
        item,  # expect: GW002
    ):
        return item


try:
    def fallback(value):  # expect: GW002 GW003
        return value
except ImportError:
    pass


key = lambda u: u  # expect: GW001


def sort_by_key(values: list[int]) -> list[int]:
    return sorted(values, key=lambda w: w)


class Shape:
    def area(self) -> float:
        return 0.0

    def scale(
        self,
        factor,  # expect: GW002
    ) -> None:
        pass

    def rename(me) -> None:
        pass

    def grow(  # expect: GW003
        self,
        by: int,
    ):
        self.size = by  # expect: GW004


def after_class(  # expect: GW003
    value,  # expect: GW002
):
    return value


def __init__(value: int):  # expect: GW003
    return value


class Registry:
    @classmethod
    def reset(cls) -> None:
        pass

    @classmethod
    def load(
        cls,
        path,  # expect: GW002
    ) -> None:
        pass

    @classmethod
    def build(cls):  # expect: GW003
        return cls()


class Maths:
    @staticmethod
    def double(
        value,  # expect: GW002
    ) -> int:
        return value * 2

    @builtins.staticmethod
    def triple(
        value,  # expect: GW002
    ) -> int:
        return value * 3


class Posonly:
    def tag(  # expect: GW003
        self,
        /,
        label,  # expect: GW002
    ):
        pass


class Splat:
    def run(  # expect: GW003
        *args,  # expect: GW002
    ):
        pass


class Client:
    async def connect(self) -> None:
        pass

    async def send(  # expect: GW003
        self,
        payload,  # expect: GW002
    ):
        pass


class Temperature:
    def __init__(self, celsius: float):
        self.celsius = celsius  # expect: GW004

    @property
    def kelvin(self) -> float:
        return self.celsius + 273.15


class Empty:
    def __init__(self):  # expect: GW003
        pass


class Holder:
    def __init__(  # expect: GW003
        self,
        x,  # expect: GW002
    ):
        self.x = x  # expect: GW004


class Packed:
    def __init__(self, *args: int):
        self.values = args  # expect: GW004


class Keyed:
    def __init__(self, **options: str):
        self.options = options  # expect: GW004


class Loose:
    def __init__(  # expect: GW003
        self,
        *args,  # expect: GW002
    ):
        self.args = args  # expect: GW004


class Mixed:
    def __init__(
        self,
        left,  # expect: GW002
        right: int,
    ):
        self.left = left  # expect: GW004


class Outer:
    def method(self) -> None:
        def helper(  # expect: GW003
            item,  # expect: GW002
        ):
            return item

        helper(1)
