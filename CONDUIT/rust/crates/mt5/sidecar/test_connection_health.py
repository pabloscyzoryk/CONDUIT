"""Read-only heartbeat through the production handler with fake dependencies."""
import unittest
from types import SimpleNamespace as NS
from test_follow_terminal import FakeMT5, mod


class ConnectionHealthTests(unittest.TestCase):
    def test_connected_disconnected_missing_and_unknown_are_distinct(self):
        fake = FakeMT5()
        mod.mt5 = fake
        sidecar = mod.Sidecar(mod.parse_args(["--port", "1"]))
        for info, expected in [(NS(connected=True), True), (NS(connected=False), False),
                               (None, False), (NS(), None), (NS(connected="false"), None)]:
            fake.terminal_info = lambda: info
            result = sidecar.cmd_ping({})
            self.assertIs(result["terminal_connected"], expected)
            self.assertEqual(set(result), {"pong", "ts", "terminal_connected"})
            self.assertTrue(result["pong"])
        self.assertEqual(fake.initializations, [])
        self.assertEqual(fake.orders_sent, [])

    def test_read_failure_is_unknown_without_initializing_or_trading(self):
        fake = FakeMT5()
        mod.mt5 = fake
        def failed():
            raise RuntimeError("synthetic health lookup failure")
        fake.terminal_info = failed
        sidecar = mod.Sidecar(mod.parse_args(["--port", "1"]))
        self.assertIsNone(sidecar.cmd_ping({})["terminal_connected"])
        self.assertEqual(fake.initializations, [])
        self.assertEqual(fake.orders_sent, [])


if __name__ == "__main__":
    unittest.main()
