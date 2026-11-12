# A type variable that a project module defines makes a subclass generic when the subclass passes it on, whether it
# is imported by name or read through the module. A name from a module outside the analysed files is not one, so
# the subclass stays exempt.
from external_types import U
from tv import Box, T
import tv


class Direct(Box[T]):
    pass


class Through(tv.Box[tv.T]):
    pass


class Unknown(Box[U]):
    pass


direct = Direct()  # expect: GW001
through = Through()  # expect: GW001
unknown = Unknown()
