# Runtime configuration templates

Copy these files next to the compiled `conduit.exe` and remove the `.example`
suffix:

- `settings.example.json` → `settings.json`
- `secrets.example.json` → `secrets.json`
- `channels.example.json` → `channels.json`
- `lancuchy.example.json` → `lancuchy.json`

Create a `presets` directory next to the executable and copy the desired JSON
files from `../presets/` into it (or copy the complete catalogue). GOD-X7 is
the current recommended reference, not the only supported preset. The application can populate Telegram API
credentials and channel bindings through its first-run UI; do not commit the
resulting runtime files.

The template starts in `MANUAL` mode and does **not** authorize real-account
trading. Test on a demo account before making either setting less restrictive.
