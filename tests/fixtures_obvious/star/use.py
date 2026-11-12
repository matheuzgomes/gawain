# A star import of a project module binds the public names that module defines. `Box` is generic there, so
# `Box()` is GW001. `_Hidden` is private, so the star import does not bind it, and `_Hidden()` stays exempt.
from box import *

empty = Box()  # expect: GW001
filled = Box(1)
hidden = _Hidden()
