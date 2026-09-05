"""Pure injected read API only: no real MetaTrader5 import or broker mutation."""
import copy
import json
import math
import unittest
from types import SimpleNamespace as NS

from pending_evidence import collect_pending_evidence


ACCOUNT = {"login": 42, "server": "fixture-demo", "trade_mode": 0}
BINDING = {"order_ticket": 700, "symbol": "XAUUSD", "magic": 770077}


def order(**changes):
    # SYNTHETIC property-complete row, NOT a measured cancellation model.
    row = dict(ticket=700, type=2, state=2, magic=770077, position_id=0,
        position_by_id=0, time_setup_msc=1700000000000, time_done_msc=1700000001000,
        symbol="XAUUSD", volume_initial=.05, volume_current=.05, price_open=3990.,
        sl=3980., tp=4010., comment="fixture")
    row.update(changes)
    return row


def deal(**changes):
    row = dict(ticket=880, order=700, type=0, entry=0, magic=770077,
        position_id=433, time_msc=1700000001500, symbol="XAUUSD", volume=.02,
        price=3990., profit=0., commission=-.1, swap=0., fee=0.)
    row.update(changes)
    return row


def position(**changes):
    row = dict(ticket=922, identifier=433, type=0, magic=770077,
        time_msc=1700000001500, symbol="XAUUSD", volume=.02, price_open=3990., sl=3980., tp=4010.)
    row.update(changes)
    return row


class FakeReadApi:
    def __init__(self):
        self.accounts = [dict(ACCOUNT), dict(ACCOUNT)]
        self.current = []
        self.history = [order()]
        self.deals = []
        self.positions = []
        self.calls = []
        self.overrides = {}
    def _read(self, name, value, kwargs=None):
        self.calls.append((name, kwargs or {}))
        out = self.overrides.get(name, value)
        if isinstance(out, Exception): raise out
        return copy.deepcopy(out)
    def account_info(self):
        index = sum(n == "account_info" for n, _ in self.calls)
        return self._read("account_info", self.accounts[min(index, len(self.accounts)-1)])
    def orders_get(self, **kwargs):
        return self._read("orders_get", self.current, kwargs)
    def history_orders_get(self, **kwargs):
        return self._read("history_orders_get", [o for o in self.history if o["ticket"] == kwargs["ticket"]], kwargs)
    def history_deals_get(self, **kwargs):
        return self._read("history_deals_get", [d for d in self.deals if d["order"] == kwargs["ticket"]], kwargs)
    def positions_get(self, **kwargs):
        return self._read("positions_get", self.positions, kwargs)
    def _forbidden(self, *args, **kwargs): raise AssertionError("FORBIDDEN broker mutation/session API")
    order_send = login = initialize = shutdown = symbol_select = _forbidden


def collect(api=None, **changes):
    args = dict(expected_account=ACCOUNT, operation_token="opaque-req-A-generation7-op9",
        local_observation_seq=51, bindings=[BINDING])
    args.update(changes)
    return collect_pending_evidence(api or FakeReadApi(), **args)


class PendingEvidenceTests(unittest.TestCase):
    def test_no_fill_raw_is_complete_but_never_qualified(self):
        r = collect()
        self.assertEqual(r["status"], "raw_complete")
        self.assertTrue(r["identity_consistent"])
        self.assertEqual(r["volume_model"], "unverified")
        self.assertIsNone(r["broker_history_read_through_msc"])
        self.assertFalse(r["app_generation_verified"])
        self.assertEqual(r["proof_status"], "not_evaluated_not_qualified")
    def test_order_filters_not_deal_filters_and_reads_only(self):
        api = FakeReadApi(); api.deals=[deal()]; collect(api)
        self.assertEqual(api.calls, [("account_info", {}), ("orders_get", {}),
            ("history_orders_get", {"ticket": 700}), ("history_deals_get", {"ticket": 700}),
            ("positions_get", {}), ("account_info", {})])
    def test_full_record_lists_and_extra_fields_are_preserved(self):
        api = FakeReadApi();api.history[0]["external_id"]="exchange-specific"
        api.current=[order(ticket=999, magic=33)];r=collect(api)
        self.assertEqual(r["orders"][0]["history"]["records"],api.history)
        self.assertEqual(r["current_orders"]["records"],api.current)
    def test_none_is_waiting_not_empty_for_every_read(self):
        for name in ["orders_get", "history_orders_get", "history_deals_get", "positions_get"]:
            api=FakeReadApi();api.overrides[name]=None;r=collect(api)
            self.assertEqual(r["status"],"waiting",name)
            reads=[r["current_orders"],r["positions"],r["orders"][0]["history"],r["orders"][0]["deals"]]
            self.assertEqual(sum(x["status"]=="none" and x["records"] is None for x in reads),1)
    def test_exception_is_review_not_empty_for_every_read(self):
        for name in ["orders_get", "history_orders_get", "history_deals_get", "positions_get"]:
            api=FakeReadApi();api.overrides[name]=RuntimeError("unavailable");r=collect(api)
            self.assertEqual(r["status"],"requires_review",name)
    def test_empty_history_remains_empty_raw_not_invented_cancel(self):
        api=FakeReadApi();api.history=[];r=collect(api)
        self.assertEqual(r["orders"][0]["history"],{"status":"complete","records":[]})
        self.assertEqual(r["proof_status"],"not_evaluated_not_qualified")
    def test_wrong_initial_account_prevents_all_history_queries(self):
        api=FakeReadApi();api.accounts[0]["login"]=99;r=collect(api)
        self.assertEqual(api.calls,[("account_info",{})])
        self.assertFalse(r["identity_consistent"])
        self.assertEqual(r["current_orders"]["status"],"not_attempted")
    def test_none_invalid_or_exception_account_never_reads_orders(self):
        for value in [None, {"login":42,"server":"","trade_mode":0}, RuntimeError("IPC")]:
            api=FakeReadApi();api.overrides["account_info"]=value;r=collect(api)
            self.assertEqual(len(api.calls),1)
            self.assertEqual(r["status"],"requires_review")
    def test_account_changed_after_reads_invalidates_whole_cut(self):
        api=FakeReadApi();api.accounts[1]["server"]="other";r=collect(api)
        self.assertEqual(r["status"],"requires_review")
        self.assertFalse(r["identity_consistent"])
        self.assertEqual(r["orders"][0]["history"]["records"],api.history)
        self.assertEqual(r["entry_position_links"],[])
    def test_local_sequence_and_echo_are_not_broker_time_or_app_identity(self):
        r=collect(local_observation_seq=7,operation_token="untrusted-echo-generation999")
        self.assertEqual(r["local_observation_seq"],7)
        self.assertEqual(r["operation_token_echo"],"untrusted-echo-generation999")
        self.assertEqual(r["max_observed_broker_timestamp_msc"],1700000001000)
        self.assertIsNone(r["broker_history_read_through_msc"])
        self.assertFalse(r["app_generation_verified"])
    def test_invalid_request_fails_before_any_read(self):
        for args in [{"local_observation_seq":True},{"local_observation_seq":0},
                     {"operation_token":""},{"bindings":[]},{"bindings":[BINDING,BINDING]},
                     {"bindings":[dict(BINDING,order_ticket=700.5)]},
                     {"expected_account":dict(ACCOUNT,login=True)}]:
            api=FakeReadApi()
            with self.assertRaises(ValueError):collect(api,**args)
            self.assertEqual(api.calls,[])
    def test_wrong_order_filter_response_is_review_and_not_filtered_away(self):
        api=FakeReadApi();api.overrides["history_deals_get"]=[deal(order=880)];r=collect(api)
        d=r["orders"][0]["deals"]
        self.assertEqual(d["status"],"invalid")
        self.assertEqual(d["records"][0]["order"],880)
    def test_missing_malformed_and_nonfinite_fields_are_not_filled_with_zero(self):
        for field,value in [("volume_current",None),("volume_initial",math.nan),("ticket",True)]:
            api=FakeReadApi();api.history[0][field]=value
            # Force malformed response through the API boundary; the normal
            # ORDER filter correctly would not return a row with ticket=True.
            api.overrides["history_orders_get"]=api.history
            r=collect(api)
            self.assertEqual(r["status"],"requires_review",field)
            self.assertEqual(len(r["orders"][0]["history"]["records"]),1)
            json.dumps(r,allow_nan=False)
        api=FakeReadApi();del api.history[0]["volume_current"];r=collect(api)
        self.assertNotIn("volume_current",r["orders"][0]["history"]["records"][0])
        self.assertEqual(r["status"],"requires_review")
    def test_invalid_extra_raw_components_are_explicit_not_complete(self):
        api=FakeReadApi();api.deals=[deal(commission=math.inf)]
        r=collect(api)
        self.assertEqual(r["status"],"requires_review")
        self.assertEqual(r["orders"][0]["deals"]["records"][0]["commission"],{"_non_finite":"inf"})
        json.dumps(r,allow_nan=False)
    def test_wrong_symbol_or_magic_in_scoped_history_is_not_owned(self):
        for changes in [{"symbol":"EURUSD"},{"magic":33}]:
            api=FakeReadApi();api.history=[order(**changes)]
            self.assertEqual(collect(api)["status"],"requires_review")
    def test_identical_duplicates_remain_visible_conflicts_mark_invalid(self):
        for conflict in [False,True]:
            api=FakeReadApi();second=order(volume_current=.02) if conflict else order()
            api.history.append(second);r=collect(api);h=r["orders"][0]["history"]
            self.assertEqual(len(h["records"]),2)
            self.assertEqual(h["status"],"invalid" if conflict else "complete")
    def test_distinct_deal_order_identifier_ticket_relation_uses_identifier(self):
        api=FakeReadApi();api.deals=[deal()];api.positions=[position(),position(ticket=433,identifier=999)]
        link=collect(api)["entry_position_links"][0]
        self.assertEqual((link["order_ticket"],link["entry_deal_ticket"],link["position_identifier"]),(700,880,433))
        self.assertEqual(link["physical_position_tickets"],[922])
    def test_no_entry_deals_yet_means_waiting_never_ticket_equals_identifier(self):
        api=FakeReadApi();api.history=[order(position_id=433,state=4,volume_current=0.)]
        api.positions=[position(ticket=700,identifier=433)]
        link=collect(api)["entry_position_links"][0]
        self.assertEqual(link["status"],"waiting_for_entry_deals")
        self.assertNotIn("physical_position_tickets",link)
    def test_fill_closed_before_position_snapshot_is_not_no_fill(self):
        api=FakeReadApi();api.deals=[deal()];api.positions=[];r=collect(api)
        self.assertEqual(r["entry_position_links"][0]["status"],"waiting_or_conflicting_current_position")
        self.assertEqual(r["entry_position_links"][0]["position_identifier"],433)
        self.assertEqual(r["volume_model"],"unverified")
    def test_partial_and_multiple_fills_are_retained_not_summed_into_no_fill(self):
        api=FakeReadApi();api.history=[order(state=3,position_id=433,volume_current=.01)]
        api.deals=[deal(ticket=881,volume=.02),deal(ticket=880,volume=.02)]
        r=collect(api)
        self.assertEqual([x["ticket"] for x in r["orders"][0]["deals"]["records"]],[881,880])
        self.assertEqual(len(r["entry_position_links"]),2)
        self.assertEqual(r["proof_status"],"not_evaluated_not_qualified")
    def test_two_order_bindings_do_not_use_one_broad_history_query(self):
        api=FakeReadApi();api.history.append(order(ticket=701));r=collect(api,bindings=[BINDING,dict(BINDING,order_ticket=701)])
        self.assertEqual(len(r["orders"]),2)
        self.assertEqual([args["ticket"] for name,args in api.calls if name=="history_deals_get"],[700,701])
    def test_named_tuple_like_rows_supported_without_real_mt5_package(self):
        api=FakeReadApi();api.overrides["history_orders_get"]=[NS(**order())]
        self.assertEqual(collect(api)["status"],"raw_complete")
    def test_bad_shape_list_keeps_invalid_raw_instead_of_empty(self):
        api=FakeReadApi();api.overrides["history_orders_get"]={"not":"a list"};r=collect(api)
        self.assertEqual(r["orders"][0]["history"]["records"],{"not":"a list"})
        self.assertEqual(r["status"],"requires_review")


if __name__ == "__main__":
    unittest.main(verbosity=2)
