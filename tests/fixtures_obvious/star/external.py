# A star import of a module outside the analysed files binds names that gawain cannot see, so the call stays exempt.
from missing_library import *

external = Box()
