class A:
    x: int


class A:
    pass


class B(A):
    def __init__(self) -> None:
        self.x = 1  # expect: GW004
