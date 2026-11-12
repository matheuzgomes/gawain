# Containers of obvious literals (GW001). A non-empty list, set, tuple or dict is obvious when its elements
# (for a dict, its keys and values) are obvious literals of one kind, or containers of one shape. A tuple
# may mix kinds. An empty container anywhere, a starred or ** element, None, or a call inside it is not.
numbers = [1, 2, 3]
names = {"a", "b"}
pair = (1, "a")
mapping = {"key": 1, "other": 2}
nested = [[1], [2]]
pairs = [(1, "a"), (2, "b")]
table = {"outer": {"inner": 1}}
signs = [-1, +2]
mixed_tuple = ([1], 2)

mixed_list = [1, 2.5]  # expect: GW001
mixed_bools = [True, 1]  # expect: GW001
mixed_set = {1, "a"}  # expect: GW001
mixed_keys = {1: "a", "b": "c"}  # expect: GW001
mixed_values = {"a": 1, "b": "c"}  # expect: GW001
nested_mixed = [[1], ["a"]]  # expect: GW001
mixed_pairs = [(1, "a"), (2, 3)]  # expect: GW001
value_lists = {"a": [1], "b": ["x"]}  # expect: GW001

empty_list = []  # expect: GW001
empty_dict = {}  # expect: GW001
empty_inner = [[]]  # expect: GW001
empty_in_tuple = (1, ())  # expect: GW001
nested_empty = {"a": {}}  # expect: GW001
none_inside = [None]  # expect: GW001
spread_list = [*numbers]  # expect: GW001
spread_dict = {**mapping}  # expect: GW001
call_inside = [len(numbers)]  # expect: GW001
comprehension = [n for n in numbers]  # expect: GW001
