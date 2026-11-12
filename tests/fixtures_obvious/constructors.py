# Constructor calls (GW001). A call to a CapWords name is obvious when it has arguments. Without arguments
# it is obvious unless the name is a known generic (Counter, Queue...) or a generic class of the project.
# A callee that is not CapWords, such as datetime(...) or MAX(), is never obvious.
import asyncio
from collections import Counter, OrderedDict, defaultdict
from datetime import datetime
from pathlib import Path
from queue import Queue
from typing import Generic, TypeVar

T = TypeVar("T")


class User:
    pass


class Box(Generic[T]):
    pass


class Bag[U]:
    pass


class Bag2(Bag[int]):
    pass


class Wrapped(Box[T]):
    pass


class Plain(Box):
    pass


class Chain(Bag2):
    pass


user_named = User("a")
user_empty = User()
user_keyword = User(name="a")
models_user = models.User()
path_value = Path("x")
cwd = Path()
url_value = URL("x")  # expect: GW001
moment = datetime(2020, 1, 1)  # expect: GW001
maximum = MAX()  # expect: GW001
word_count = Counter(["a"])
empty_counter = Counter()  # expect: GW001
ordered = OrderedDict()  # expect: GW001
grouped = defaultdict(list)  # expect: GW001
pending = Queue()  # expect: GW001
pending_future = asyncio.Future()  # expect: GW001
empty_box = Box()  # expect: GW001
filled_box = Box(1)
empty_bag = Bag()  # expect: GW001
empty_bag2 = Bag2()
wrapped_box = Wrapped()  # expect: GW001
plain_box = Plain()  # expect: GW001
chain_bag = Chain()
typed_box = Box[int]()  # expect: GW001
built_widget = factory().Widget()


# A shared ancestor is read once, and a cycle of bare bases ends the walk. Neither class is generic.
class Root:
    pass


class Left(Root):
    pass


class Right(Root):
    pass


class Diamond(Left, Right):
    pass


class LoopA(LoopB):
    pass


class LoopB(LoopA):
    pass


# A bare base from outside the project, such as Exception, adds no type parameters.
class Failure(Exception):
    pass


diamond = Diamond()
loop = LoopA()
failure = Failure()
