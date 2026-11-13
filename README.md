<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/matheuzgomes/gawain/main/docs/assets/gawain-wordmark-dark.svg">
    <img src="https://raw.githubusercontent.com/matheuzgomes/gawain/main/docs/assets/gawain-wordmark.svg" alt="gawain: the green axe with the green girdle, above the name in blackletter" width="200">
  </picture>
</p>

gawain requires type annotations on the variables, parameters, returns and class attributes of Python code,
then runs a type checker, [ty](https://docs.astral.sh/ty/) or [pyrefly](https://pyrefly.org/), to check that the
types are correct.

Type checkers such as mypy, Pyright, ty and pyrefly infer the type of a variable that has no annotation. ruff's `ANN`
rules and flake8-annotations ask for annotations on functions only. gawain also asks for them on variables and
class attributes.

## Why

I built gawain only because I hate `dict[str, Any]` and wanted to do something about it. If your answer to
this is "but Python already infers that" or "but my IDE infers it for me", then don't use it, little padawan.

## What it reports

Given `shop.py`:

```python
PORT = 8080
retries = 3
total = compute()


def send(to, body):
    print(f"to {to}: {body}")


class Order:
    def __init__(self, items: list[str]) -> None:
        self.items = items
```

`gawain check shop.py` prints:

```text
[presence]
shop.py:1:1    GW001 variable 'PORT' has no type annotation
shop.py:3:1    GW001 variable 'total' has no type annotation
shop.py:6:5    GW003 function 'send' has no return type annotation
shop.py:6:10   GW002 parameter 'to' of function 'send' has no type annotation
shop.py:6:14   GW002 parameter 'body' of function 'send' has no type annotation
shop.py:12:14  GW004 attribute 'items' is not declared in the body of class 'Order'

6 errors (presence). Type check skipped until presence errors are fixed.
```

`retries = 3` passes, because a literal has an obvious type. `PORT` is a constant, so it needs `Final[int]`.

When every annotation is in place, gawain runs the type checker on the same files and prints what it reports.
With ty, the default, for
`prices.py`:

```python
def total(prices: list[int]) -> int:
    return "0"
```

```text
[types]
prices.py:2:12  ty[invalid-return-type] Return type does not match returned value: expected `int`, found `Literal["0"]`

1 error (types).
```

## Install

Install gawain with the extra of the type checker you want. ty is the default checker, and the `ty` extra pins
it to 0.0.100:

```sh
uv tool install "gawain[ty]"
# or
pip install "gawain[ty]"
```

To use pyrefly instead, install the `pyrefly` extra, which pins pyrefly 1.4.1 and does not install ty:

```sh
uv tool install "gawain[pyrefly]"
# or
pip install "gawain[pyrefly]"
```

Then choose it with `checker = "pyrefly"` in `[tool.gawain]`, or with `--checker pyrefly`.
[docs/configuration.md](https://github.com/matheuzgomes/gawain/blob/main/docs/configuration.md#choosing-the-type-checker)
explains the switch.

`pip install gawain` without an extra installs only the `gawain` command. It then needs a checker on `PATH`, or
`--no-types` to run the annotation checks only.

> [!WARNING]
> Use gawain with care. ty is in beta, and its diagnostics may change between versions. If that is a problem
> for your project, use pyrefly, or `--no-types` to run the annotation checks only.

## Usage

```sh
gawain check src/                  # annotations, then types
gawain check src/ --no-types       # annotations only
gawain check src/ --checker pyrefly  # pyrefly instead of ty
gawain check src/ --mode strict    # an annotation on every assignment
gawain check src/ --format json    # one JSON object, for tools
```

gawain exits with `0` when nothing is reported, `1` when it reports an error, and `2` on a usage error or
when the type checker cannot run. The type checker's warnings are printed but do not fail the check.

## Rules

| Code  | Reports                                                                                   |
| ----- | ----------------------------------------------------------------------------------------- |
| GW001 | A variable assigned with no annotation.                                                   |
| GW002 | A parameter with no annotation.                                                           |
| GW003 | A function with no return annotation.                                                     |
| GW004 | A class attribute with no annotation, or a `self.x` that the class body does not declare. |
| GW010 | A constant that is not annotated `Final[T]`.                                              |
| GW011 | A bare `Final`, with no type.                                                             |
| GW020 | `Any` in an annotation.                                                                   |

GW000 marks a file gawain cannot read, and GW900 and GW901 mark a broken or unused suppression. To silence a
line, name the code:

```python
value = load()  # gawain: ignore[GW001]
```

[docs/rules.md](https://github.com/matheuzgomes/gawain/blob/main/docs/rules.md) has the full rules, the exemptions and the suppression syntax.

## Modes

The default mode is `obvious`. A name may go without an annotation when its value has an obvious type and the
name is never assigned again: a literal (`count = 0`), a container of literals (`ids = [1, 2, 3]`) or a
constructor call (`user = User("ada")`). `strict` asks for an annotation on every assignment:

```toml
[tool.gawain]
mode = "strict"
```

[docs/rules.md](https://github.com/matheuzgomes/gawain/blob/main/docs/rules.md#modes) has the exact rules of each mode.

## pre-commit

```yaml
repos:
  - repo: https://github.com/matheuzgomes/gawain
    rev: v0.3.0
    hooks:
      - id: gawain
```

pre-commit installs the hook from this repository's source, so building it needs Rust 1.97 or newer.
[docs/configuration.md](https://github.com/matheuzgomes/gawain/blob/main/docs/configuration.md) covers `[tool.gawain]`, paths and the hook in detail.

## Performance

On the CPython 3.14 standard library (1,847 files, 974,586 lines), median of 30 runs (5 for flake8-annotations):

| Tool                 | Checks                                     | Median |
| -------------------- | ------------------------------------------ | ------ |
| gawain, obvious mode | variables, parameters, returns, attributes | 0.46 s |
| gawain, strict mode  | variables, parameters, returns, attributes | 0.36 s |
| ruff `--select ANN`  | parameters, returns                        | 0.35 s |
| flake8-annotations   | parameters, returns                        | 11.4 s |

gawain ran with `--no-types`. This is one benchmark on one machine (AMD Ryzen 7 3700X, WSL2).
[docs/benchmarks.md](https://github.com/matheuzgomes/gawain/blob/main/docs/benchmarks.md) has the versions and the commands to run it again.

## Documentation

- [docs/rules.md](https://github.com/matheuzgomes/gawain/blob/main/docs/rules.md): rules, modes, exemptions and suppression.
- [docs/configuration.md](https://github.com/matheuzgomes/gawain/blob/main/docs/configuration.md): paths, output, exit codes, `[tool.gawain]` and pre-commit.
- [docs/benchmarks.md](https://github.com/matheuzgomes/gawain/blob/main/docs/benchmarks.md): how the numbers above were measured.

## Acknowledgements

gawain reads Python with [ruff_python_parser](https://crates.io/crates/ruff_python_parser) and checks types with
ty, both by [Astral](https://astral.sh), or with [pyrefly](https://pyrefly.org/), by Meta.

## Name

I don't know, I just like the Arthurian tales. Gawain is a knight of the Round Table, and the mascot is
the Green Knight's axe with the green girdle from _Sir Gawain and the Green Knight_.

## License

MIT. See [LICENSE](https://github.com/matheuzgomes/gawain/blob/main/LICENSE).
