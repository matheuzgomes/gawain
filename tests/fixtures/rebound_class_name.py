# A class that a later import rebinds: the import is the last binding, so the base is external.
class A:
    pass


from external_lib import A


class B(A):
    def __init__(self) -> None:
        self.x = 1


# An import that a later class rebinds: the class is the last binding, so its base declares nothing.
from external_lib import E


class E:
    pass


class F(E):
    def __init__(self) -> None:
        self.x = 1  # expect: GW004
