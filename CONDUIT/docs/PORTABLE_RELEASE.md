# Portable Windows release

The release carries its own x64 Python runtime and sidecar dependencies.
`START_CONDUIT.vbs` opens the native application from the package directory.
`START_BROWSER.vbs` starts the same local application with its browser UI.
The executable also works directly; its default data directory is beside it.
The separate `postep.exe` progress window is included and can be opened by the
application's laboratory when a backtest starts.

The private VPS package follows the already signed-in MT5 terminal. Its
Telegram authorization comes from the explicitly selected private template.
The public package starts in manual mode with empty channel bindings and no
authorization material. The build and verification commands start no services.

## Reproducible runtime

```powershell
python tools/portable_runtime.py --destination <new-runtime-directory> --downloads <archive-cache>
python tools/portable_runtime.py --destination <runtime-directory> --verify
```

The build pins CPython 3.13.15, MetaTrader5 5.0.5735 and NumPy 2.5.1 for
Windows x64. It checks published archive hashes before extraction, retains
upstream licenses and wheel metadata, and tests imports using the bundled
interpreter. It never calls `MetaTrader5.initialize` or sends an order.

Upstream distributions: [CPython](https://www.python.org/downloads/release/python-31315/),
[MetaTrader5](https://pypi.org/project/metatrader5/5.0.5735/),
[NumPy](https://pypi.org/project/numpy/2.5.1/).

## Reviewed strategy and account contract

`tools/package_release.py stage` requires a selection document containing the
selected preset ID and hash, the recorded owner selection, all chain limits,
and an explicit account overlay. Economic account fields override per-format
settings in the live application; the package cannot silently inherit an old
position limit, basket limit, lot basis or cost assumption from a VPS template.
The selected recipe must already include these fields in its validation.

Every release records public file hashes, the selected strategy, chain limits
and account overlay. `BIEZACY.json`, the active Synergy leg, the settings and
package stamp must agree. Public manifests carry no credential values or
hashes derived from private identities.

```powershell
python tools/package_release.py stage --source-root CONDUIT --private-template <VPSREADY-template> --executable <conduit.exe> --monitor <postep.exe> --runtime <verified-runtime> --preset <selected.json> --selection <selection.json> --kind public --destination <new-public-release-directory>
python tools/package_release.py verify --package <release-directory>
```

For a private build use `--kind private` and a new destination under a
`VPSREADY` directory. Private verification additionally takes the explicit
`--private-template` and compares authorization material in memory. It reports
presence and equality, never the private values. Existing destinations are not
overwritten. A failed staging directory remains marked `INCOMPLETE`.

Source export uses an immutable Git commit and an explicit file allowlist.
It excludes private VPS folders, local research inputs, logs, caches and build
outputs. The one permitted secrets example is checked for empty identities.

Offline package verification proves payload integrity and configuration
agreement. Successful live authentication is recorded separately only after
it has actually been observed.
