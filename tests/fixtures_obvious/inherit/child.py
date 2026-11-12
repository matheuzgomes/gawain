from base import Base


class Child(Base):
    def __init__(self) -> None:
        self.count = 1
        self.limit = 2
        self.extra = 3  # expect: GW004
