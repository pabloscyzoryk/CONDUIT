"""Strict ORDER->DEAL identity and receipt-only ACK; wholly fake MT5.

Imports the self-contained mock foundation next to this file. No private data,
real terminal, credentials, network connection, or order dispatch is used.
"""
from __future__ import annotations
import hashlib
import json
import math
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

# Embedded Python may intentionally omit the script directory from sys.path.
# Add only this synthetic fixture directory, never a global environment path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_sidecar_history_order_identity as old

FAKE = old.FAKE
SIDECAR = old.SIDECAR


def subject(strict=True, ack=None):
    s = old.subject(strict, ack)
    s.request_account = None
    s.account_checks = []
    s._ensure_account = lambda expected=None: s.account_checks.append(expected)
    return s


def new_deal(*args, **kwargs):
    d = old.deal(*args, **kwargs)
    d.fee = -.1
    return d


def new_position(*args, **kwargs):
    p = old.position(*args, **kwargs)
    p.magic = 770077
    return p


def opening(s=None):
    return (s or subject()).cmd_open_market({"side": "buy", "volume": .05})


class ExactAckTests(unittest.TestCase):
    def setUp(self):
        FAKE.rows = [new_deal()]
        FAKE.position_rows = [new_position()]
        FAKE.lookups = []
        self.sleep_patch = patch.object(SIDECAR.time, "sleep", lambda _: None)
        self.sleep_patch.start()

    def tearDown(self):
        self.sleep_patch.stop()

    def helper(self, s=None):
        return (s or subject())._exact_ack_deal(700, 880, "XAUUSD", 0, (0,),
            ack_volume=.05, ack_price=4000.0)

    def test_order_not_deal_query_and_distinct_physical_ticket(self):
        s = subject()
        ack = opening(s)
        self.assertEqual((ack["order"], ack["deal"], ack["position_identifier"], ack["position"]),
                         (700, 880, 433, 922))
        self.assertEqual(FAKE.lookups, [{"ticket": 700}])
        self.assertEqual(len(s.account_checks), 4)

    def test_another_order_with_deal_number_does_not_hijack_identity(self):
        FAKE.rows.append(new_deal(ticket=990, order=880, identifier=555))
        FAKE.position_rows.append(new_position(ticket=999, identifier=555))
        self.assertEqual(opening()["position_identifier"], 433)

    def test_multiple_order_fills_are_selected_by_exact_ack_deal_in_any_order(self):
        own = new_deal()
        sibling = new_deal(ticket=879, identifier=432)
        for rows in [[sibling, own], [own, sibling]]:
            FAKE.rows = rows
            self.assertEqual(self.helper().position_id, 433)

    def test_no_ack_deal_never_guesses_from_order(self):
        ack = types.SimpleNamespace(retcode=10009, order=700, deal=0,
            volume=.05, price=4000.0, comment="mock ack")
        got = opening(subject(ack=ack))
        self.assertEqual((got["position_identifier"], got["position"]), (0, 0))
        self.assertEqual(FAKE.lookups, [])

    def test_missing_order_never_uses_broad_history(self):
        self.assertIsNone(subject()._exact_ack_deal(0, 880, "XAUUSD", 0, (0,),
            ack_volume=.05, ack_price=4000.0))
        self.assertEqual(FAKE.lookups, [])

    def test_later_history_availability_retries_reads_only(self):
        for first in [None, (), RuntimeError("history temporarily unavailable")]:
            count = []
            def delayed(*args, **kwargs):
                count.append(kwargs)
                if len(count) == 1:
                    if isinstance(first, Exception): raise first
                    return first
                return (new_deal(),)
            with patch.object(FAKE, "history_deals_get", delayed):
                self.assertEqual(self.helper().position_id, 433)
            self.assertEqual(count, [{"ticket": 700}, {"ticket": 700}])

    def test_permanent_none_and_exception_produce_unknown_not_fake_identity(self):
        for result in [None, (), RuntimeError("history unavailable")]:
            count = []
            def missing(*args, **kwargs):
                count.append(kwargs)
                if isinstance(result, Exception): raise result
                return result
            with patch.object(FAKE, "history_deals_get", missing):
                got = opening()
            self.assertEqual((got["position_identifier"], got["position"]), (0, 0))
            self.assertEqual(len(count), 20)

    def test_wrong_symbol_side_entry_magic_or_identifier_never_confirms(self):
        for field, value in [("symbol", "EURUSD"), ("type", 1), ("entry", 1),
                             ("magic", 4), ("position_id", 0)]:
            d = new_deal()
            setattr(d, field, value)
            FAKE.rows = [d]
            self.assertIsNone(self.helper(), field)

    def test_invalid_deal_volume_or_price_never_confirms(self):
        for field in ["volume", "price"]:
            for value in [0, -1, math.nan, math.inf]:
                d = new_deal(); setattr(d, field, value); FAKE.rows = [d]
                self.assertIsNone(self.helper(), (field, value))

    def test_ack_geometry_conflict_does_not_confirm_open_identity(self):
        for field, value in [("volume", .01), ("price", 3999.0)]:
            d = new_deal(); setattr(d, field, value); FAKE.rows = [d]
            got = opening()
            self.assertEqual((got["order"], got["deal"]), (700, 880))
            self.assertEqual((got["position_identifier"], got["position"]), (0, 0), field)

    def test_ack_geometry_must_be_finite_and_positive(self):
        for field in ["volume", "price"]:
            for value in [0.0, -1.0, math.nan, math.inf, True]:
                ack = types.SimpleNamespace(retcode=10009, order=700, deal=880,
                    volume=.05, price=4000.0, comment="mock ack")
                setattr(ack, field, value)
                self.assertEqual(opening(subject(ack=ack))["position_identifier"], 0,
                                 (field, value))

    def test_aggregate_volume_cannot_be_guessed_from_multiple_fills(self):
        first = new_deal(ticket=879); first.volume=.02
        exact = new_deal(); exact.volume=.03
        FAKE.rows = [first, exact]
        self.assertEqual(opening()["position_identifier"], 0)

    def test_ack_geometry_conflict_does_not_confirm_close_identity_or_book_profit(self):
        for field, value in [("volume", .01), ("price", 3999.0)]:
            d = new_deal(ticket=881, order=701, entry=1, profit=7.0)
            setattr(d, field, value); FAKE.rows = [d]
            got = self.close()
            self.assertEqual((got["order"], got["deal"], got["position"]), (701, 881, 922))
            self.assertEqual(got["position_identifier"], 0, field)
            self.assertFalse(got["ack_identity_complete"])
            self.assertIsNone(got["ack_deal_components"])
            self.assertEqual(got["profit"], 0.0)

    def test_current_position_volume_is_not_mistaken_for_ack_deal_volume(self):
        FAKE.position_rows[0].volume = .12
        self.assertEqual(opening()["position_identifier"], 433)

    def test_geometry_tolerance_is_only_four_binary_ulps_not_lot_or_tick_rounding(self):
        for field, original in [("volume", .05), ("price", 4000.0)]:
            for ulps, confirmed in [(4, True), (5, False)]:
                d = new_deal()
                setattr(d, field, original + ulps * math.ulp(original))
                FAKE.rows = [d]
                self.assertEqual(opening()["position_identifier"], 433 if confirmed else 0,
                                 (field, ulps))

    def test_duplicate_exact_or_conflicting_duplicate_is_unknown(self):
        for rows in [[new_deal(), new_deal()],
                     [new_deal(), new_deal(identifier=555)]]:
            FAKE.rows = rows
            self.assertIsNone(self.helper())

    def test_wrong_order_in_response_is_not_silently_filtered(self):
        with patch.object(FAKE, "history_deals_get", lambda **kwargs:
                          (new_deal(), new_deal(ticket=991, order=701))):
            self.assertIsNone(self.helper())

    def test_identifier_and_enum_values_are_not_truncated_from_float_or_bool(self):
        for field, value in [("ticket", 880.2), ("order", 700.2), ("position_id", 433.2),
                             ("type", .2), ("entry", False), ("ticket", True)]:
            d = new_deal(); setattr(d, field, value); FAKE.rows = [d]
            self.assertIsNone(self.helper(), (field, value))
        for order, deal in [(700.2, 880), (700, 880.2), (True, 880), (700, True)]:
            self.assertIsNone(subject()._exact_ack_deal(order, deal, "XAUUSD", 0, (0,),
                ack_volume=.05, ack_price=4000.0))

    def test_missing_or_ambiguous_current_physical_ticket_never_guesses(self):
        for rows in [[], [new_position(), new_position(ticket=999)], None]:
            with patch.object(FAKE, "positions_get", lambda **kwargs: rows):
                got = opening()
            self.assertEqual(got["position_identifier"], 433)
            self.assertEqual(got["position"], 0)

    def test_current_physical_position_has_to_be_owned_and_same_symbol_side(self):
        for field, value in [("magic", 4), ("symbol", "EURUSD"), ("type", 1),
                             ("ticket", 0), ("volume", math.nan)]:
            p = new_position(); setattr(p, field, value)
            with patch.object(FAKE, "positions_get", lambda **kwargs: [p]):
                self.assertEqual(opening()["position"], 0, field)

    def test_account_guard_exception_is_not_swallowed_as_an_empty_history(self):
        s = subject()
        def changed(expected=None):
            raise SIDECAR.BrokerError(SIDECAR.ERR_ACCOUNT_CHANGED, "changed")
        s._ensure_account = changed
        with self.assertRaises(SIDECAR.BrokerError): opening(s)
        self.assertEqual(FAKE.lookups, [])

    def close(self, strict=True):
        ack = types.SimpleNamespace(retcode=10009, order=701, deal=881,
            volume=.05, price=4002.0, comment="mock close")
        return subject(strict, ack)._close(922, .05)

    def test_close_ack_is_receipt_only_not_canonical_net_or_second_booking(self):
        FAKE.rows.append(new_deal(ticket=881, order=701, entry=1, profit=7.0))
        got = self.close()
        self.assertEqual((got["position_identifier"], got["position"], got["deal"]), (433, 922, 881))
        self.assertEqual(got["profit"], 0.0)
        self.assertEqual(got["profit_basis"], "receipt_only_no_realized_in_ack_v1")
        self.assertTrue(got["ack_identity_complete"])
        self.assertEqual(got["ack_deal_components"],
            {"gross_profit": 7.0, "exit_commission": -.2, "swap": -.3, "exit_fee": -.1})
        s = subject(); s.poll_deals()
        self.assertEqual(s.frames[0]["profit"], 7.0)
        self.assertEqual(s.frames[0]["position"], 433)

    def test_close_unknown_exact_deal_preserves_ack_ids_but_triggers_existing_identity_gate(self):
        got = self.close()
        self.assertEqual((got["order"], got["deal"], got["position"]), (701, 881, 922))
        self.assertEqual(got["position_identifier"], 0)
        self.assertFalse(got["ack_identity_complete"])
        self.assertEqual(got["profit"], 0.0)
        self.assertIsNone(got["ack_deal_components"])

    def test_close_cannot_accept_exact_deal_from_another_position(self):
        FAKE.rows.append(new_deal(ticket=881, order=701, identifier=555, entry=1, profit=7.0))
        self.assertEqual(self.close()["position_identifier"], 0)

    def test_close_does_not_truncate_malformed_position_identifier(self):
        FAKE.rows.append(new_deal(ticket=881, order=701, entry=1, profit=7.0))
        FAKE.position_rows[0].identifier = 433.5
        self.assertEqual(self.close()["position_identifier"], 0)

    def test_partial_close_ack_keeps_actual_returned_volume_and_does_not_book_estimate(self):
        d = new_deal(ticket=881, order=701, entry=1, profit=2.0); d.volume=.02
        FAKE.rows.append(d)
        ack = types.SimpleNamespace(retcode=10010, order=701, deal=881,
            volume=.02, price=4002.0, comment="mock partial")
        got = subject(ack=ack)._close(922, .04)
        self.assertEqual(got["volume"], .02)
        self.assertEqual(got["profit"], 0.0)
        self.assertEqual(got["position_identifier"], 433)

    def test_missing_cost_component_is_not_assumed_zero_even_in_diagnostic(self):
        d = new_deal(ticket=881, order=701, entry=1, profit=7.0); del d.fee
        FAKE.rows.append(d)
        got = self.close()
        self.assertTrue(got["ack_identity_complete"])
        self.assertIsNone(got["ack_deal_components"])
        self.assertEqual(got["profit"], 0.0)

    def test_off_open_golden_preserves_legacy_queries_and_response_bytes(self):
        for collision in [False, True]:
            FAKE.rows = [new_deal()]
            if collision: FAKE.rows.append(new_deal(ticket=990, order=880, identifier=555))
            FAKE.lookups = []
            got = opening(subject(False))
            legacy_identifier = 555 if collision else 0
            expected = {"retcode":10009,"order":700,"deal":880,"position":legacy_identifier,
                "position_identifier":legacy_identifier,"volume":.05,"price":4000.0,"comment":"mock ack"}
            self.assertEqual(json.dumps(got, separators=(",", ":")),
                             json.dumps(expected, separators=(",", ":")))
            self.assertEqual(FAKE.lookups, [{"ticket":880}] * (1 if collision else 20))

    def test_off_close_golden_preserves_legacy_estimate_even_when_lookup_is_wrong(self):
        for collision in [False, True]:
            FAKE.rows = [new_deal(), new_deal(ticket=881, order=701, entry=1, profit=7.0)]
            if collision: FAKE.rows.append(new_deal(ticket=999, order=881, entry=1, profit=9.0))
            FAKE.lookups = []
            got = self.close(False)
            expected = {"retcode":10009,"order":701,"deal":881,"position":922,
                "volume":.05,"position_identifier":433,"price":4002.0,
                "profit":8.5 if collision else 0.0,"comment":"mock close"}
            self.assertEqual(json.dumps(got, separators=(",", ":")),
                             json.dumps(expected, separators=(",", ":")))
            self.assertEqual(FAKE.lookups, [{"ticket":881}])


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--emit-bridge-fixture":
        # Consumed by the new Rust actual-Bridge test. No test runner/real MT5.
        variant = sys.argv[2] if len(sys.argv) > 2 else "complete"
        FAKE.rows = [] if variant == "unknown_open" else [new_deal()]
        if variant == "geometry_open":
            FAKE.rows[0].volume=.01
            FAKE.rows[0].price=3999.0
        FAKE.position_rows = [new_position()]
        with patch.object(SIDECAR.time, "sleep", lambda _: None):
            open_ack = opening()
            if variant != "unknown_close":
                FAKE.rows.append(new_deal(ticket=881, order=701, entry=1, profit=7.0))
                if variant == "geometry_close":
                    FAKE.rows[-1].volume=.01
                    FAKE.rows[-1].price=3999.0
            ack = types.SimpleNamespace(retcode=10009, order=701, deal=881,
                volume=.05, price=4002.0, comment="mock close")
            close_ack = subject(ack=ack)._close(922,.05)
        print(json.dumps({"open_ack":open_ack, "close_ack":close_ack,
            "source_sha256":hashlib.sha256(old.SOURCE.read_bytes()).hexdigest()}))
    else:
        print(json.dumps({"source_sha256":hashlib.sha256(old.SOURCE.read_bytes()).hexdigest(),
            "mode":"strict-correctness-and-OFF-golden", "real_mt5_imported":False}), flush=True)
        unittest.main(verbosity=2)
