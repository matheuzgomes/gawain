# Configuration

How gawain finds files, what it prints, its exit codes, `[tool.gawain]`, the choice of type checker and the
pre-commit hook.

## Usage

```sh
gawain check PATHS... [--no-types] [--format text|json] [--mode obvious|strict] [--checker ty|pyrefly]
```

- `PATHS` are Python files or directories. A directory is walked, and `.gitignore` is honoured. A path
  named here is always checked, even when `exclude` lists it (see [Configuration](#configuration)).
  A symbolic link to a Python file that the walk finds is checked under the link's own path. A symbolic
  link to a directory is not followed, and a link that points to nothing gives a GW000. A file reached
  by more than one path is checked once, under the first path in sorted order.
- `--no-types` skips the types phase.
- `--format json` prints one object with a `checker` name, a `presence` array and a `types` array. `checker`
  and `types` are `null` when the types phase did not run. In `types`, each diagnostic has a `severity` of
  `error` or `warning`.
- `--mode` chooses the mode of the presence phase, see [Modes](rules.md#modes). It overrides `mode` in the config.
- `--checker` chooses the type checker of the types phase, see
  [Choosing the type checker](#choosing-the-type-checker). It overrides `checker` in the config.

When the paths hold no Python file, gawain prints `gawain: no Python files found in the given paths` to
stderr and exits 0, unless a path could not be read.

Exit codes:

- `0`: nothing reported, or the type checker reported only warnings.
- `1`: a presence diagnostic, or an error from the type checker.
- `2`: a usage error, an unreadable config, an unknown key in `[tool.gawain]`, an unknown mode or checker (in
  the command line or in the config), or a type checker that cannot run.

## Configuration

Set `exclude`, `mode` and `checker` in `pyproject.toml`:

```toml
[tool.gawain]
exclude = ["tests/fixtures"]
mode = "obvious"
checker = "ty"
```

The paths are relative to the working directory. `exclude` applies to paths found by walking a directory.
A path named on the command line, file or directory, is always checked. So `gawain check .` skips
`tests/fixtures`, while `gawain check tests/fixtures` checks it.

`mode` is `obvious` (the default) or `strict`, see [Modes](rules.md#modes). `--mode` on the command line overrides it.

`checker` is `ty` (the default) or `pyrefly`, see [Choosing the type checker](#choosing-the-type-checker).
`--checker` on the command line overrides it.

`[tool.gawain]` accepts only the keys above. Any other key, such as `exlcude`, is a usage error (exit 2), so
a misspelt setting cannot pass silently.

## Choosing the type checker

The types phase runs ty or pyrefly. ty is the default. Each checker comes with an extra of its own: `ty`
pins ty 0.0.100 and `pyrefly` pins pyrefly 1.4.1. gawain without an extra installs neither.

To switch to pyrefly:

1. Install gawain with the `pyrefly` extra, which pins pyrefly 1.4.1 and does not install ty:

   ```sh
   uv tool install "gawain[pyrefly]"
   # or
   pip install "gawain[pyrefly]"
   ```

2. Choose pyrefly in `pyproject.toml`:

   ```toml
   [tool.gawain]
   checker = "pyrefly"
   ```

   Or choose it for one run, which also overrides the config:

   ```sh
   gawain check src/ --checker pyrefly
   ```

To switch back, install the `ty` extra (`pip install "gawain[ty]"`), then remove the key or set
`checker = "ty"`. `--checker ty` overrides `checker = "pyrefly"` for one run.

gawain looks for the checker's executable (`ty` or `pyrefly`, with `.exe` on Windows) first in the directory of
the `gawain` executable, where pip and uv install it, and then on `PATH`. When it is found in neither place,
gawain exits with code 2 and names the package to install.

Each checker reads its own configuration: ty reads `[tool.ty]`, and pyrefly reads `pyrefly.toml` or
`[tool.pyrefly]`. The rules and severities are theirs. gawain prints each diagnostic as `ty[rule]` or
`pyrefly[rule]`. Errors fail the types phase, and warnings are printed and counted but do not fail it.

How gawain runs each one:

- ty: `ty check --output-format gitlab --exit-zero-on-warning -- FILES`.
- pyrefly: `pyrefly check --output-format json --summary=none --min-severity warn -- FILES`. Without a
  configuration, pyrefly uses its `basic` preset, which leaves out most checks, so it would not report
  `x: int = "a"`. So when none of the checked files finds a `pyrefly.toml`, or a `pyproject.toml` with a
  `[tool.pyrefly]` table, in its directory or a directory above it, gawain adds `--preset default`. When a
  configuration is found, gawain adds nothing, and the preset of that configuration applies.

## pre-commit

```yaml
repos:
  - repo: https://github.com/matheuzgomes/gawain
    rev: v0.2.0
    hooks:
      - id: gawain
```

The hook uses `language: python`, so pre-commit installs gawain from this repository's source and builds it.
That needs a Rust toolchain (1.97 or newer) and network access. The hook installs ty 0.0.100 with it. To use
pyrefly in the hook, replace that dependency and choose pyrefly in `[tool.gawain]`:

```yaml
repos:
  - repo: https://github.com/matheuzgomes/gawain
    rev: v0.2.0
    hooks:
      - id: gawain
        additional_dependencies: ["pyrefly==1.4.1"]
```

pre-commit passes the staged file names to gawain, so `exclude` in `[tool.gawain]` does not apply there. To
leave files out of the hook, use pre-commit's own `exclude:` option:

```yaml
repos:
  - repo: https://github.com/matheuzgomes/gawain
    rev: v0.2.0
    hooks:
      - id: gawain
        exclude: ^tests/fixtures/
```

The hook runs serially, and pre-commit may split a long list of files into several runs. Class resolution
across files sees only the files of one run. A class whose base is defined in a file of another run is
treated as having an external base: attributes set through `self` are then not checked, and an Enum
subclass of such a base may have its plain members reported. To see every cross-file case, run
`gawain check` on the whole tree in one run, for example in CI.
