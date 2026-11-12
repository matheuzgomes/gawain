# An obvious class-body assignment that no method assigns is exempt, and it declares the attribute for the
# subclass in the other file of this directory.
class Base:
    count = 0
    limit = 10

    def run(self) -> None:
        return None
