from .reexport import Base


class Derived(Base):
    def __init__(self) -> None:
        self.name = "derived"
        self.other = 1  # expect: GW004
