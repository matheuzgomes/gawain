# A class-body assignment that a method also assigns is not exempt, so it declares nothing for the subclass.
# The one GW004 is on the class-body line, and the method's assignment is not reported again.
class Parent:
    count = 0  # expect: GW004

    def bump(self) -> None:
        self.count = 1
