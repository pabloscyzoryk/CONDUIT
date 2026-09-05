# Quick-backtest screening

These helpers generate and run a deterministic, checkpointed candidate pool.
They are research tools, not a replacement for the exact backtest.

`--quick-tick-stride N` with `N > 1` preserves selected extrema and mandatory
event ticks while sampling the remaining tick stream. Its output is explicitly
marked approximate and is never eligible for preset coronation. Every finalist
must be rerun with `--quick-tick-stride 1` and the ordinary exact validation
suite before it can be considered for a release.

Typical flow from the repository root:

```powershell
python narzedzia/quick_backtest/prepare_quick_sweep.py --help
python narzedzia/quick_backtest/run_quick_sweep.py --help
```

The public snapshot intentionally contains no tick file, Telegram export,
previous sweep manifest or generated result. Supply your own inputs through the
documented command-line options. Generated work belongs under `work/`, which is
ignored by Git.
