# Benchmarks

How the numbers in the README were measured. This is one benchmark on one machine. Run it on yours before you
rely on it.

## Results

| Tool | Runs | Median | Min | Max | Lines of output |
| --- | --- | --- | --- | --- | --- |
| gawain, obvious mode | 30 | 0.463 s | 0.444 s | 0.491 s | 182,174 |
| gawain, strict mode | 30 | 0.364 s | 0.354 s | 0.392 s | 201,353 |
| ruff `--select ANN` | 30 | 0.349 s | 0.340 s | 0.381 s | 105,094 |
| flake8-annotations | 5 | 11.406 s | 11.351 s | 11.477 s | 159,445 |

The tools do not check the same things, so the output sizes differ. ruff and flake8-annotations check
parameters and returns. gawain also checks variables and class attributes.

gawain ran with `--no-types`, so the times cover the presence phase only. ty is not part of this comparison.

## Code

The standard library of CPython 3.14, from Homebrew's `python@3.14` (`lib/python3.14`): 1,852 `.py` files. Five
files under `test/` were removed, because some of the tools in the first run of this benchmark stop with an
error on them:

- `test/encoded_modules/module_iso_8859_1.py`
- `test/encoded_modules/module_koi8_r.py`
- `test/test_tarfile.py`
- `test/tokenizedata/badsyntax_3131.py`
- `test/tokenizedata/badsyntax_pep3120.py`

That leaves 1,847 files and 974,586 lines.

## Machine and versions

- AMD Ryzen 7 3700X (8 cores, 16 threads), 15 GiB of memory available to WSL2, Linux 6.6.
- gawain 0.1.0, release build.
- ruff 0.17.4.
- flake8 7.5.0 with flake8-annotations 3.3.1.

## Method

Each command runs from the root of the code once as a warm-up, then the number of times in the table. Each
run is timed from start to exit, with its output sent to a pipe, and the table gives the median, the minimum
and the maximum.

The commands:

```sh
gawain check --no-types .
gawain check --no-types --mode strict .
ruff check --no-cache --select ANN --output-format concise .
flake8 --select ANN .
```
