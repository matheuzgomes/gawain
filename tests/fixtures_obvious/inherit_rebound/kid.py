from parent import Parent


class Kid(Parent):
    def __init__(self) -> None:
        self.count = 2  # expect: GW004
