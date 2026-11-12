# A first binding is exempt only when its name is never rebound in the same scope. An augmented assignment
# and a read do not rebind. Each rebinding form below makes the first assignment a GW001.
count = 0  # expect: GW001
count = count + 1
hits = 0
hits += 1

for_total = 0  # expect: GW001
for for_total in range(3):
    pass

with_handle = ""  # expect: GW001
with open("notes.txt") as with_handle:
    pass

except_name = 0  # expect: GW001
try:
    pass
except ValueError as except_name:
    pass

walrus_value = 0  # expect: GW001
if (walrus_value := 2) > 1:
    pass

sep_text = ""  # expect: GW001
from os import sep as sep_text

declared_later = 0  # expect: GW001
declared_later: int

pair_head = 1  # expect: GW001
pair_head, pair_tail = (2, 3)

match_value = 0  # expect: GW001
match 1:
    case match_value:
        pass

handler = 1  # expect: GW001


def handler() -> None:
    pass


mode = "wide"  # expect: GW001
if mode:
    mode = "narrow"

label = "a"
print(label)


def run_tally() -> int:
    tally = 0  # expect: GW001

    def bump() -> None:
        nonlocal tally
        tally = tally + 1

    bump()
    return tally


def count_steps() -> int:
    steps = 0

    def step() -> None:
        nonlocal steps
        steps += 1

    step()
    return steps


def read_only() -> int:
    base = 2

    def double() -> int:
        return base * 2

    return double()


def run() -> None:
    value = 0  # expect: GW001
    value = value + 1
    other = 0
    other += 1


def deep_tally() -> int:
    deep = 0  # expect: GW001

    def middle() -> None:
        def leaf() -> None:
            nonlocal deep
            deep = 1

        leaf()

    middle()
    return deep


total_hits = 0  # expect: GW001


def record_hit() -> None:
    global total_hits
    total_hits = total_hits + 1


removed = 0  # expect: GW001
del removed
removed = 1

lambda_total = 0
apply_lambda = lambda: (lambda_total := 1)  # expect: GW001

comp_total = 0  # expect: GW001
doubled = [comp_total := n for n in range(3)]  # expect: GW001

comp_target = 0
squares = [comp_target for comp_target in range(3)]  # expect: GW001

method_total = 0  # expect: GW001


class Owner:
    def bump(self) -> None:
        global method_total
        method_total = 1


def run_frame() -> None:
    frame_value = 0  # expect: GW001

    class Frame:
        frame_value = 5

        def step(self) -> None:
            nonlocal frame_value
            frame_value = 1

    Frame().step()


async def drain(source: object) -> None:
    item = 0  # expect: GW001
    async for item in source:
        pass


async def hold(lock: object) -> None:
    handle = ""  # expect: GW001
    async with lock as handle:
        pass
