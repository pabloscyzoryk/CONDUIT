# Native progress monitor

`postep.exe` displays running backtests, training, research metadata, completed
preset results and the local task queue. It reads the existing progress-file
protocol; switching language does not rename fields, tasks or presets.

The top bar provides **EN / PL**. A choice is saved as `language.txt` in the
selected progress directory. Startup preference is:

1. `postep.exe --language en` or `--language pl`
2. `CONDUIT_LANGUAGE=en` or `pl`
3. The saved `language.txt`
4. English

Both `--language=en` and `--language en` are accepted. An invalid explicit CLI
language is rejected. A missing or invalid optional environment/file preference
falls through to the next source. A save failure leaves the current selection
active and displays a small warning beside the selector.

`CONDUIT_POSTEP_DIR` and the existing portable `postep-dir.txt` retain their
directory-selection rules. The per-directory process lock is unchanged.
The monitor never reads account settings or secrets to determine language.
Callers that already have a language setting can pass it through
`uruchom_okno_z_jezykiem(Some("en"))`; the original launcher API remains available.

Static controls, explanations, research-validation labels, ranking criteria,
known units and number formatting support both languages. User task names,
preset names, commands, custom metric names, process logs and producer-provided
free-text status remain as supplied.
