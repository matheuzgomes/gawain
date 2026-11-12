# Line suppressions. A suppression comment silences the listed codes on its own line and nothing else.
# A malformed one is reported as GW900, and an unused one as GW901. Neither can be suppressed. Lines without a marker report nothing.
from typing import Any

a = 1  # gawain: ignore[GW001]
b = 1  # gawain: ignore[GW002]  # expect: GW001 GW901
c = 1  # gawain: ignore  # expect: GW001 GW900
d = 1  # gawain: ignore[]  # expect: GW001 GW900
e = 1  # gawain: ignore[GW999, GW001]  # expect: GW900
g2 = 1  # noqa  # gawain: ignore[GW001]
h = 1  # gawain:ignore[ GW001 ]
i = 1  # gawain: ignore[gw001]  # expect: GW001 GW900
j = 1  # gawain: ignore[GW900]  # expect: GW001 GW900
k1 = 1  # gawain ignore[GW001]  # expect: GW001
l1 = 1  # gawain: ignored  # expect: GW001
m1 = 1  # gawain : ignore[GW001]
m2 = 1  # gawain:  ignore[ GW001 ,  GW002 ]  # expect: GW901
m3 = 1  # gawain: ignore[GW001  # expect: GW001 GW900
m4 = 1  # gawain: ignore[GW001,]
n1 = 1  # mygawain: ignore[GW001]  # expect: GW001
p1 = 1  #gawain: ignore[GW001]
p2 = 1  # x#gawain: ignore[GW001]
s: str = "# gawain: ignore"
u = "# gawain: ignore[GW001]"  # expect: GW001
v = 1  # expect: GW001
# gawain: ignore[GW001]  # expect: GW901
w = 1  # expect: GW001
bundle = print(  # expect: GW001
    "x",  # gawain: ignore[GW001]  # expect: GW901
)


def f(x): ...  # gawain: ignore[GW002,GW003]


def g(x): ...  # gawain: ignore[GW002]  # expect: GW003


def h2(
    value: Any,  # gawain: ignore[GW020]
) -> None:
    return None


def k(x): ...  # gawain: ignore[GW002, GW003]


def g(x): ...  # gawain: ignore[GW002,GW003,GW001]  # expect: GW901


def ok(x: int) -> int: ...  # gawain: ignore[GW002]  # expect: GW901


r1 = 1  # gawain: ignore[GW002, GW002]  # expect: GW001 GW901
s1 = 1  # gawain: ignore[GW999, GW002]  # expect: GW001 GW900 GW901
t1 = 1  # gawain: ignore[GW901]  # expect: GW001 GW900
