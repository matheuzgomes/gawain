from app.models import Base as B2
from .models import Base


class User(Base):
    def __init__(self) -> None:
        self.name = ""
        self.extra = 1  # expect: GW004


class Member(B2):
    def __init__(self) -> None:
        self.name = "member"
        self.level = 2  # expect: GW004
