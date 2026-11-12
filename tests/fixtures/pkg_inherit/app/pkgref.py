import app.models
import app.models as models_alias
from . import models


class Ref(models.Base):
    def __init__(self) -> None:
        self.name = "ref"
        self.bad = 1  # expect: GW004


class Alias(models_alias.Base):
    def __init__(self) -> None:
        self.name = "alias"


class Chain(app.models.Base):
    def __init__(self) -> None:
        self.name = "chain"
