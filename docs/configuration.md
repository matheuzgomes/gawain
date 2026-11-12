# Configuration

How gawain finds files, what it prints, its exit codes, `[tool.gawain]` and the pre-commit hook.

## Usage

```sh
gawain check PATHS... [--no-types] [--format text|json] [--mode obvious|strict]
```

- `PATHS` are Python files or directories. A directory is walked, and `.gitignore` is honoured. A path
  named here is always checked, even when `exclude` lists it (see [Configuration](#configuration)).
  A symbolic link to a Python file that the walk finds is checked under the link's own path. A symbolic
  link to a directory is not followed, and a link that points to nothing gives a GW000. A file reached
  by more than one path is checked once, under the first path in sorted order.
- `--no-types` skips the types phase.
- `--format json` prints one object with a `presence` array and a `types` array (`null` when the types
  phase did not run).
- `--mode` chooses the mode of the presence phase, see [Modes](rules.md#modes). It overrides `mode` in the config.

When the paths hold no Python file, gawain prints `gawain: no Python files found in the given paths` to
stderr and exits 0, unless a path could not be read.

Exit codes:

- `0`: nothing reported, or ty reported only warnings.
- `1`: a presence diagnostic, or a ty error.
- `2`: a usage error, an unreadable config, an unknown key in `[tool.gawain]`, or an unknown mode (in the
  command line or in the config), or ty that cannot run.

## Configuration

Set `exclude` and `mode` in `pyproject.toml`:

```toml
[tool.gawain]
exclude = ["tests/fixtures"]
mode = "obvious"
```

The paths are relative to the working directory. `exclude` applies to paths found by walking a directory.
A path named on the command line, file or directory, is always checked. So `gawain check .` skips
`tests/fixtures`, while `gawain check tests/fixtures` checks it.

`mode` is `obvious` (the default) or `strict`, see [Modes](rules.md#modes). `--mode` on the command line overrides it.

`[tool.gawain]` accepts only the keys above. Any other key, such as `exlcude`, is a usage error (exit 2), so
a misspelt setting cannot pass silently.

## pre-commit

```yaml
repos:
  - repo: https://github.com/matheuzgomes/gawain
    rev: v0.1.0
    hooks:
      - id: gawain
```

The hook uses `language: python`, so pre-commit installs gawain from this repository's source and builds it.
That needs a Rust toolchain (1.97 or newer) and network access.

pre-commit passes the staged file names to gawain, so `exclude` in `[tool.gawain]` does not apply there. To
leave files out of the hook, use pre-commit's own `exclude:` option:

```yaml
repos:
  - repo: https://github.com/matheuzgomes/gawain
    rev: v0.1.0
    hooks:
      - id: gawain
        exclude: ^tests/fixtures/
```

The hook runs serially, and pre-commit may split a long list of files into several runs. Class resolution
across files sees only the files of one run. A class whose base is defined in a file of another run is
treated as having an external base: attributes set through `self` are then not checked, and an Enum
subclass of such a base may have its plain members reported. To see every cross-file case, run
`gawain check` on the whole tree in one run, for example in CI.
