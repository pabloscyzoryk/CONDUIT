"""Actual Sidecar producer with fake MetaTrader5; never real login/order_send."""
import copy
import unittest
from types import SimpleNamespace as NS
from cost_contract import cost_receipt_payload
from test_cost_contract import deal
import test_follow_terminal as follow


class CostPayloadTests(unittest.TestCase):
    def test_causal_fingerprint_and_cutoff_ignore_future_and_sort_input(self):
        a=deal(1,0,.08,commission=-.56,fee=-.08,ts=100)
        x=deal(2,1,.02,commission=-.04,fee=-.01,swap=-.5,profit=2.,ts=200)
        future=deal(3,0,999,commission=-9999,ts=200)
        p=cost_receipt_payload(x,[x,a],"USD")
        self.assertTrue(p["complete"])
        self.assertEqual(p,cost_receipt_payload(x,[future,a,x,a],"USD"))
        self.assertEqual(p["cutoff_time_msc"],200)
        self.assertEqual(p["first_entry_time_msc"],100)
        self.assertEqual(len(p["history_fingerprint"]),64)

    def test_missing_fields_and_inout_are_not_zero_net(self):
        a=deal(1,0,.08,commission=-.56,fee=-.08)
        x=deal(2,1,.08,profit=4.)
        for key in ("profit","commission","fee","swap"):
            missing=copy.deepcopy(x);del missing[key]
            p=cost_receipt_payload(missing,[a,missing],"USD")
            self.assertFalse(p["complete"],key)
        p=cost_receipt_payload(x,None,"USD")
        self.assertFalse(p["complete"]);self.assertFalse(p["history_query_complete"])
        self.assertFalse(cost_receipt_payload(x,[a,x],"")["complete"])
        rev=deal(2,2,.08)
        self.assertFalse(cost_receipt_payload(rev,[a,rev],"USD")["complete"])

    def test_partial_signed_allocations_match_full_cash_components(self):
        a=deal(1,0,.07,commission=-.49,fee=-.03)
        xs=[deal(2,1,.02,commission=-.1,fee=-.01,swap=-.2,profit=2.),
            deal(3,1,.02,commission=.02,fee=.01,swap=.1,profit=1.),
            deal(4,1,.03,commission=-.05,fee=0.,swap=-.1,profit=3.)]
        ps=[cost_receipt_payload(x,[a]+xs,"USD") for x in xs]
        self.assertTrue(all(p["complete"] for p in ps))
        self.assertAlmostEqual(sum(p["entry_commission_alloc"] for p in ps),-.49)
        self.assertAlmostEqual(sum(p["entry_fee_alloc"] for p in ps),-.03)
        keys=("gross_profit","entry_commission_alloc","exit_commission","entry_fee_alloc","exit_fee","swap")
        self.assertAlmostEqual(sum(sum(p[k] for k in keys) for p in ps),5.15)


class ActualSidecarCostTests(unittest.TestCase):
    def setUp(self):
        self.fake=follow.FakeMT5();follow.mod.mt5=self.fake
        self.fake.account.currency="USD"
        self.history=[deal(1,0,.08,commission=-.56,fee=-.08,ts=100),
                      deal(2,1,.08,commission=-.24,fee=-.01,swap=-6.,profit=4.,ts=200)]
        for row in self.history:
            row.update(magic=777,symbol="XAUUSD",price=4000.,comment="fixture",reason=3)
        self.exit=NS(**self.history[-1]);self.frames=[];self.requests=[];self.switch=False
        def history(*args,**kwargs):
            self.requests.append(kwargs)
            if "position" in kwargs:
                if self.switch:self.fake.account.server="other-server"
                return None if self.history is None else tuple(NS(**r) for r in self.history)
            return (self.exit,)
        self.fake.history_deals_get=history

    def sidecar(self,on=True):
        args=follow.mod.parse_args(["--port","1","--magic","777","--close-receipt-reconcile"]
                                  +(["--closed-profit-net-costs"] if on else []))
        s=follow.mod.Sidecar(args);s.bound_account=follow.mod.account_key(self.fake.account)
        s.send=self.frames.append;return s

    def test_actual_poll_emits_complete_separate_proof_once_no_order_send(self):
        s=self.sidecar();s.poll_deals();s.poll_deals()
        self.assertEqual(len(self.frames),1)
        p=self.frames[0]["cost_receipt"]
        self.assertTrue(p["complete"]);self.assertEqual(p["exit_fee"],-.01)
        self.assertEqual(self.frames[0]["profit"],4.) # cash/gross not rewritten in producer
        self.assertEqual(self.fake.orders_sent,[])

    def test_off_wire_does_not_gain_cost_field_or_extra_history_query(self):
        s=self.sidecar(False);s.poll_deals()
        self.assertNotIn("cost_receipt",self.frames[0])
        self.assertEqual(sum("position" in q for q in self.requests),1)

    def test_missing_history_and_missing_original_field_are_explicit_incomplete(self):
        self.history=None;s=self.sidecar();s.poll_deals()
        self.assertFalse(self.frames[0]["cost_receipt"]["complete"])
        self.assertIsNone(self.frames[0]["cost_receipt"]["entry_commission_alloc"])

    def test_missing_original_commission_is_not_certified_by_legacy_zero(self):
        del self.history[-1]["commission"];self.exit=NS(**self.history[-1])
        s=self.sidecar();s.poll_deals()
        self.assertEqual(self.frames[0]["commission"],0.)
        self.assertIsNone(self.frames[0]["cost_receipt"]["exit_commission"])
        self.assertFalse(self.frames[0]["cost_receipt"]["complete"])

    def test_actual_inout_is_explicit_incomplete_on_and_unchanged_skip_off(self):
        self.history[-1]["entry"]=2;self.exit=NS(**self.history[-1])
        s=self.sidecar();s.poll_deals()
        self.assertEqual(len(self.frames),1,"ON must not silently discard a reversal cost event")
        self.assertFalse(self.frames[0]["cost_receipt"]["complete"])
        self.assertEqual(self.frames[0]["cost_receipt"]["incomplete_reason"],"not_supported_exit")
        self.frames.clear();s=self.sidecar(False);s.poll_deals()
        self.assertEqual(self.frames,[])

    def test_actual_null_or_nonnumeric_raw_cost_emits_incomplete_instead_of_throwing_after_seen(self):
        for field in ("profit","commission","swap"):
            for invalid in (None,"unknown",float("nan"),float("inf"),True):
                self.setUp()
                self.history[-1][field]=invalid;self.exit=NS(**self.history[-1])
                s=self.sidecar();s.poll_deals()
                self.assertEqual(len(self.frames),1,(field,invalid))
                self.assertEqual(self.frames[0][field],0.) # placeholder cannot certify the separate None
                self.assertFalse(self.frames[0]["cost_receipt"]["complete"])
                proof_key={"profit":"gross_profit","commission":"exit_commission","swap":"swap"}[field]
                self.assertIsNone(self.frames[0]["cost_receipt"][proof_key])

    def test_account_switch_during_history_lookup_does_not_emit_other_account_proof(self):
        s=self.sidecar();self.switch=True
        with self.assertRaises(follow.mod.BrokerError):s.poll_deals()
        self.assertEqual(self.frames,[]);self.assertTrue(s.account_changed)

    def test_costs_require_receipt_identity_before_any_terminal_initialize(self):
        args=follow.mod.parse_args(["--port","1","--closed-profit-net-costs"])
        with self.assertRaises(ValueError):follow.mod.Sidecar(args)
        self.assertEqual(self.fake.initializations,[])


if __name__=="__main__":unittest.main()
