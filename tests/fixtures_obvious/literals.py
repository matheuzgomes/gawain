# Obvious literals at module and function scope (GW001). A first binding of an obvious literal needs no
# annotation. None, Ellipsis, a t-string, a unary operator on a name, or any other expression does.
int_value = 1
float_value = 1.5
complex_value = 2j
str_value = "text"
concat_value = "a" "b"
bytes_value = b"raw"
bool_value = True
empty_text = ""
fstring_value = f"{int_value}"
negative_int = -1
positive_float = +2.5
negative_complex = -3j

none_value = None  # expect: GW001
ellipsis_value = ...  # expect: GW001
template_value = t"hello {int_value}"  # expect: GW001
negated_name = -int_value  # expect: GW001
inverted = ~1  # expect: GW001
negated_flag = not True  # expect: GW001


def literals_in_function() -> None:
    local_int = 1
    local_text = "x"
    local_none = None  # expect: GW001
