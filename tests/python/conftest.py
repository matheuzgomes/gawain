"""Puts the repository root on sys.path, so that the tests import scripts/compare.py as scripts.compare.

The path is taken without resolving symlinks. The mutation runner links the tests into a worker
directory and copies the mutated script next to them, so a resolved path would load the original.
"""

import sys
from pathlib import Path
from typing import Final

ROOT: Final[Path] = Path(__file__).absolute().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
