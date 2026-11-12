from .base import Base


class Box(Base):
    def __init__(self, size: int) -> None:
        self.size = size
