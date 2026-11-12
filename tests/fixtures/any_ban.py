# Any anywhere in a type expression (GW020). Every occurrence of Any that resolves to typing.Any is one
# diagnostic, placed at its start. Every other line is annotated and declared, so the only diagnostics
# here are the marked GW020 ones.
import sys
import typing
import typing as t
import typing_extensions
from typing import Annotated, Any, Callable, Literal, Optional, TypeAlias, cast
from typing import Any as A

if sys.version_info >= (3, 12):
    from typing import Any as Modern

x: Any = 1  # expect: GW020
y: dict[str, Any] = {}  # expect: GW020
z: dict[str, object] = {}


def f(a: Any) -> Any:  # expect: GW020 GW020
    return a


def varargs(*args: Any, **kw: Any) -> None:  # expect: GW020 GW020
    pass


cb: Callable[..., Any] = print  # expect: GW020
union: int | Any = 1  # expect: GW020
meta: Annotated[Any, "m"] = 1  # expect: GW020
quoted: "Any" = 1  # expect: GW020
nested_quote: list["Any"] = []  # expect: GW020
optional_quote: Optional["dict[str, Any]"] = None  # expect: GW020
deep_quote: "list['Any']" = []  # expect: GW020
broken: "list[Any" = []
dotted: typing.Any = 1  # expect: GW020
alias_module: t.Any = 1  # expect: GW020
plain_attr: typing.Optional[int] = None
extension: typing_extensions.Any = 1  # expect: GW020
renamed: A = 1  # expect: GW020
versioned: Modern = 1  # expect: GW020
type Alias = list[Any]  # expect: GW020
Alias2: TypeAlias = dict[str, Any]  # expect: GW020


def g[T: Any](v: T) -> T:  # expect: GW020
    return v


class Defaults[T: Any = Any, *Ts = *tuple[Any, ...], **P = [Any]]:  # expect: GW020 GW020 GW020 GW020
    pass


class Holder:
    v: Any  # expect: GW020
    w: int = 0

    def store(self) -> None:
        self.w: Any = 1  # expect: GW020
        inner: Any = 2  # expect: GW020


def local_import() -> None:
    from typing import Any as Inner
    import typing as local_typing

    deep: Inner = 1  # expect: GW020
    deeper: local_typing.Any = 2  # expect: GW020


def uses_cast(v: int) -> int:
    return cast(Any, v)


lit: Literal["Any"] = "Any"
meta_ok: Annotated[int, "Any"] = 1
single: Annotated[Any] = 1  # expect: GW020
as_value: object = list[Any]
