"""Offline-only account-switch regression. Never imports the real MetaTrader5 package."""
import importlib.util
import json
import os
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace as NS
from unittest.mock import patch


class FakeMT5:
    def __init__(self):
        self.account = NS(login=42, server="Vantage-Demo", company="Vantage", trade_mode=0)
        self.symbols = {"XAUUSD": self.si(), "XAUUSD.s": None}
        self.initializations = []
        self.orders_sent = []
        self.lookup_hook = None
        self.send_hook = None

    @staticmethod
    def si():
        return NS(trade_mode=4, digits=2, point=.01, trade_contract_size=100,
                  volume_min=.01, volume_step=.01, filling_mode=1, trade_stops_level=0)

    def account_info(self):
        return self.account

    def initialize(self, path=None, /, **kwargs):
        assert "path" not in kwargs, "SDK path is positional"
        self.initializations.append(dict(kwargs, **({"path": path} if path else {})))
        return True

    def symbol_info(self, symbol):
        return self.symbols.get(symbol)

    def symbol_select(self, symbol, visible):
        return symbol in self.symbols

    def symbol_info_tick(self, symbol):
        return NS(bid=3000., ask=3000.2)

    def positions_get(self, **kwargs):
        if self.lookup_hook:
            self.lookup_hook()
        return [NS(ticket=7, volume=.08, type=0, symbol="XAUUSD")]

    def orders_get(self, **kwargs):
        if self.lookup_hook:
            self.lookup_hook()
        return [NS(ticket=7, symbol="XAUUSD")]

    def order_send(self, req):
        self.orders_sent.append(dict(req))
        if self.send_hook:
            return self.send_hook()
        return NS(retcode=10009, order=7, deal=0, comment="fixture", price=3000., volume=.04)


fake = FakeMT5()
sys.modules["MetaTrader5"] = fake
spec = importlib.util.spec_from_file_location("follow_sidecar_fixture", Path(__file__).with_name("mt5_sidecar.py"))
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


class FollowAccountTests(unittest.TestCase):
    def setUp(self):
        global fake
        fake = FakeMT5()
        mod.mt5 = fake

    def sidecar(self, follow=True, allow=False):
        args = mod.parse_args(["--port", "1", "--login", "999", "--server", "OLD-REAL"]
                              + (["--follow-terminal-account"] if follow else [])
                              + (["--allow-real-account"] if allow else []))
        s = mod.Sidecar(args)
        with patch.object(mod, "running_terminal_path", return_value="C:/running/terminal64.exe"):
            s.init_terminal()
        s.request_account = dict(s.bound_account) if s.bound_account else None
        return s

    def dispatch(self, s, cmd, args):
        replies = []
        s.send = replies.append
        s.handle_line(json.dumps({"id": 1, "cmd": cmd, "args": args}).encode())
        return replies[-1]

    def switch(self, server="Vantage-Other", mode=0):
        fake.account = NS(login=42, server=server, company="Vantage", trade_mode=mode)

    def test_follow_initialize_has_path_only_despite_old_credentials(self):
        with patch.dict(os.environ, {"CONDUIT_MT5_PASSWORD": "fixture"}):
            self.sidecar()
        self.assertEqual(fake.initializations, [{"path": "C:/running/terminal64.exe"}])

    def test_off_preserves_legacy_login(self):
        with patch.dict(os.environ, {"CONDUIT_MT5_PASSWORD": "fixture"}):
            self.sidecar(follow=False)
        self.assertEqual(fake.initializations[0], {"login": 999, "password": "fixture", "server": "OLD-REAL"})

    def test_no_process_or_ambiguous_process_fails_closed(self):
        for paths, requested in [([], None), (["C:/a/terminal64.exe", "C:/b/terminal64.exe"], None),
                                 (["C:/a/terminal64.exe"], "C:/b/terminal64.exe")]:
            with self.subTest(paths=paths), self.assertRaises(mod.BrokerError):
                mod.select_running_terminal(paths, requested)
        self.assertTrue(mod.select_running_terminal(["C:/a/terminal64.exe"]).endswith("terminal64.exe"))

    def test_process_discovery_delegates_native_errors_without_initializing(self):
        import terminal_discovery
        with patch.object(terminal_discovery, "running_terminal_path", return_value="C:/a/terminal64.exe") as query:
            self.assertTrue(mod.running_terminal_path().endswith("terminal64.exe"))
            query.assert_called_once_with(None)
        with patch.object(terminal_discovery, "running_terminal_path",
                          side_effect=terminal_discovery.TerminalDiscoveryError("fixture discovery refused")):
            with self.assertRaises(mod.BrokerError) as failure:
                mod.running_terminal_path()
            self.assertEqual(failure.exception.code, mod.ERR_NOT_INITIALIZED)
            self.assertEqual(str(failure.exception), "fixture discovery refused")
        self.assertEqual(fake.initializations, [])

    def test_known_broker_disambiguates_two_full_contracts(self):
        fake.symbols["XAUUSD.s"] = fake.si()
        self.assertEqual(self.sidecar().symbol, "XAUUSD")
        fake.account.company = "PU Prime"
        fake.account.server = "PUPrime-PUBLIC-DEMO"
        self.assertEqual(self.sidecar().symbol, "XAUUSD.s")
        fake.account.company = "unknown"
        fake.account.server = "unknown"
        with self.assertRaises(mod.BrokerError):
            self.sidecar()

    def test_disabled_or_invalid_contract_is_not_selected(self):
        fake.symbols["XAUUSD"].trade_mode = 3
        with self.assertRaises(mod.BrokerError):
            self.sidecar()
        fake.symbols["XAUUSD"].trade_mode = 4
        fake.symbols["XAUUSD"].volume_step = 0
        with self.assertRaises(mod.BrokerError):
            self.sidecar()

    def test_same_login_different_server_is_latched_even_after_switch_back(self):
        s = self.sidecar()
        expected = dict(s.bound_account)
        self.switch()
        with self.assertRaises(mod.BrokerError) as e:
            s._ensure_account(expected, mutation=True)
        self.assertEqual(e.exception.code, -6)
        self.switch("Vantage-Demo")
        with self.assertRaises(mod.BrokerError):
            s._ensure_account(expected, mutation=True)
        self.assertEqual(fake.orders_sent, [])

    def test_same_login_server_but_trade_mode_change_blocked(self):
        s = self.sidecar()
        fake.account.trade_mode = 2
        with self.assertRaises(mod.BrokerError):
            s._guarded_order_send({"position": 7})
        self.assertEqual(fake.orders_sent, [])

    def test_real_requires_explicit_consent_and_missing_envelope_never_mutates(self):
        fake.account.trade_mode = 2
        s = self.sidecar()
        with self.assertRaises(mod.BrokerError) as e:
            s._guarded_order_send({})
        self.assertEqual(e.exception.code, -7)
        s = self.sidecar(allow=True)
        s._guarded_order_send({})
        self.assertEqual(len(fake.orders_sent), 1)
        s.request_account = None
        with self.assertRaises(mod.BrokerError):
            s._guarded_order_send({})
        self.assertEqual(len(fake.orders_sent), 1)

    def test_ticket_collision_between_lookup_and_mutation_blocks_all_ticket_commands(self):
        commands = [("modify_position", {"ticket": 7, "sl": 2990, "tp": 3020}),
                    ("modify_pending", {"ticket": 7, "price": 2999}),
                    ("cancel_pending", {"ticket": 7}),
                    ("close_position", {"ticket": 7}),
                    ("close_partial", {"ticket": 7, "volume": .04})]
        for cmd, args in commands:
            with self.subTest(cmd=cmd):
                self.setUp()
                s = self.sidecar()
                args = dict(args, _expected_account=dict(s.bound_account))
                fake.lookup_hook = self.switch
                result = self.dispatch(s, cmd, args)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error"]["code"], -6)
                self.assertEqual(fake.orders_sent, [])

    def test_invalid_fill_retry_rechecks_account(self):
        s = self.sidecar()
        def refused_then_switch():
            self.switch()
            return NS(retcode=10030, comment="invalid fill")
        fake.send_hook = refused_then_switch
        with self.assertRaises(mod.BrokerError) as e:
            s._send_order({"action": 1}, [0, 1])
        self.assertEqual(e.exception.code, -6)
        self.assertEqual(len(fake.orders_sent), 1)

    def test_entry_market_and_pending_require_current_identity(self):
        for cmd, args in [("open_market", {"side":"buy","volume":.01}),
                          ("place_pending", {"kind":2,"volume":.01,"price":2999.})]:
            with self.subTest(cmd=cmd):
                self.setUp()
                s = self.sidecar()
                result = self.dispatch(s, cmd, args)
                self.assertFalse(result["ok"])
                self.assertEqual(fake.orders_sent, [])
                args["_expected_account"] = dict(s.bound_account)
                result = self.dispatch(s, cmd, args)
                self.assertTrue(result["ok"])
                self.assertEqual(len(fake.orders_sent), 1)

    def test_streams_carry_account_and_changed_account_drops_stream(self):
        s = self.sidecar()
        class Socket:
            def __init__(self): self.lines = []
            def sendall(self, data): self.lines.append(json.loads(data))
        s.sock = Socket()
        s.send({"ev":"closed_foreign","ticket":7})
        self.assertEqual(s.sock.lines[0]["account"], s.bound_account)
        self.switch()
        s.send({"ev":"tick","ts":1,"bid":3000,"ask":3001})
        self.assertEqual(len(s.sock.lines),1)

    def test_initialization_recovery_does_not_rebind_existing_sidecar(self):
        s = self.sidecar()
        self.switch()
        with patch.object(mod,"running_terminal_path",return_value="C:/running/terminal64.exe"):
            with self.assertRaises(mod.BrokerError) as e:
                s.init_terminal()
        self.assertEqual(e.exception.code,-6)
        self.assertTrue(all(set(k) == {"path"} for k in fake.initializations))

    def test_transparent_sidecar_restart_does_not_accept_previous_account_rpc(self):
        old = self.sidecar()
        old_identity = dict(old.bound_account)
        self.switch()
        fresh = self.sidecar()
        result = self.dispatch(fresh, "cancel_pending", {"ticket": 7, "_expected_account": old_identity})
        self.assertFalse(result["ok"])
        self.assertEqual(result["error"]["code"], -6)
        self.assertEqual(fake.orders_sent, [])

    def test_all_production_order_send_calls_go_through_one_guard(self):
        import ast
        tree = ast.parse(Path(mod.__file__).read_text(encoding="utf-8-sig"))
        calls = [n for n in ast.walk(tree) if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
                 and isinstance(n.func.value, ast.Name) and n.func.value.id == "mt5" and n.func.attr == "order_send"]
        self.assertEqual(len(calls), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
