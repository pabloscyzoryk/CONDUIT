"""Run the real sidecar entry loop with a stub installed BEFORE module loading."""
import importlib.util
import sys
import time
from pathlib import Path
from types import SimpleNamespace as NS

mode = sys.argv[sys.argv.index("--symbol") + 1]
if mode == "IMPORT_FAIL":
    print("ImportError: synthetic unavailable dependency fixture-password", file=sys.stderr)
    sys.exit(7)


class FakeMT5:
    def initialize(self, path=None, /, **kwargs):
        assert not kwargs, "fixture must never accept path or credentials as keyword"
        assert path == (r"c:\fixture\terminal64.exe" if mode.startswith("FOLLOW") else None)
        if mode == "SLOW_INIT":
            time.sleep(16.0)
        return mode != "FAIL_INIT"

    def last_error(self):
        return (-10003, "fixture initialize rejected")

    def symbol_select(self, *_):
        return True

    def symbol_info(self, *_):
        if mode.startswith("FOLLOW") and _[0] != "XAUUSD":
            return None
        return NS(point=.01, digits=2, trade_stops_level=0, filling_mode=1,
                  trade_mode=4, trade_contract_size=100, volume_min=.01, volume_step=.01)

    def account_info(self):
        return NS(login=42, server="synthetic-demo", trade_mode=0)

    def terminal_info(self):
        return NS(connected=True, trade_allowed=True, tradeapi_disabled=False)

    def version(self):
        return (5, 1, "fixture")

    def history_deals_get(self, *_):
        return ()

    def symbol_info_tick(self, *_):
        return NS(time_msc=1, bid=3000., ask=3000.2)

    def shutdown(self):
        pass

    def order_send(self, *_):
        raise AssertionError("No order API may run in startup fixtures")


sys.modules["MetaTrader5"] = FakeMT5()
source = Path(__file__).resolve().parents[2] / "sidecar" / "mt5_sidecar.py"
sys.path.insert(0, str(source.parent))
spec = importlib.util.spec_from_file_location("actual_startup_sidecar", source)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
# Exercise the production discovery selector/wrapper; only the native Windows
# enumeration boundary is substituted. The fixture never inspects terminals.
import terminal_discovery
class Inventory:
    def session(self, pid):
        return 1
    def terminals(self, session):
        if mode == "FOLLOW_ABSENT":
            return []
        return [{"pid": 7, "session": 1, "path": "C:/fixture/terminal64.exe"}]
terminal_discovery.WindowsProcesses = Inventory
sys.exit(mod.main())
