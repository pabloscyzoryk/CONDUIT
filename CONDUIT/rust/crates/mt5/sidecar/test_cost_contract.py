"""Pure synthetic unit tests; never imports MetaTrader5 or runs order_send."""
import copy
import math
import unittest
from cost_contract import closed_costs


def deal(ticket, entry, volume, *, commission=0.0, fee=0.0, swap=0.0, profit=0.0, ts=None, side=None):
    return dict(ticket=ticket, position_id=100, time_msc=ticket if ts is None else ts,
        entry=entry, type=(0 if entry == 0 else 1) if side is None else side,
        volume=volume, commission=commission, fee=fee, swap=swap, profit=profit)


class ClosedCostContractTests(unittest.TestCase):
    def test_full_signed_costs(self):
        a = deal(1, 0, .08, commission=-.56, fee=-.08)
        z = deal(2, 1, .08, commission=-.24, fee=-.01, swap=-6.0, profit=4.0)
        r = closed_costs(z, [a, z])
        self.assertTrue(r["complete"])
        self.assertAlmostEqual(r["net_profit"], -2.89)

    def test_partials_allocate_entry_once_and_exact_residue(self):
        a = deal(1, 0, .07, commission=-.49, fee=-.03)
        exits = [deal(2, 1, .02), deal(3, 1, .02), deal(4, 1, .03)]
        rows = [closed_costs(z, [a]+exits) for z in exits]
        self.assertTrue(all(r["complete"] for r in rows))
        self.assertAlmostEqual(sum(r["entry_commission_alloc"] for r in rows), -.49)
        self.assertAlmostEqual(sum(r["entry_fee_alloc"] for r in rows), -.03)
        self.assertAlmostEqual(rows[0]["entry_commission_alloc"], -.14)
        self.assertAlmostEqual(rows[2]["entry_commission_alloc"], -.21)

    def test_restart_is_same_stateless_result(self):
        h = [deal(1, 0, .08, commission=-.56),deal(2, 1, .04),deal(3, 1, .04)]
        self.assertEqual(closed_costs(h[-1], h), closed_costs(copy.deepcopy(h[-1]), copy.deepcopy(h)))

    def test_multiple_entry_fills_and_entry_after_partial(self):
        h = [deal(1, 0, .02, commission=-.14),deal(2, 0, .02, commission=-.28),
             deal(3, 1, .02),deal(4, 0, .02, commission=-.42),deal(5, 1, .04)]
        first, last = closed_costs(h[2],h),closed_costs(h[-1],h)
        self.assertAlmostEqual(first["entry_commission_alloc"], -.21)
        self.assertAlmostEqual(last["entry_commission_alloc"], -.63)

    def test_future_information_cannot_change_target(self):
        h = [deal(1,0,.02,commission=-.14),deal(2,1,.01)]
        initial = closed_costs(h[-1],h)
        self.assertEqual(initial, closed_costs(h[-1],h+[deal(3,0,9.0,commission=-9999),deal(4,2,2.0)]))

    def test_same_timestamp_tie_uses_deal_id(self):
        a,z,future = deal(1,0,.02,commission=-.14,ts=10),deal(2,1,.01,ts=10),deal(3,0,.02,commission=-9,ts=10)
        self.assertAlmostEqual(closed_costs(z,[future,z,a])["entry_commission_alloc"],-.07)

    def test_positive_swap_and_rebates_preserved(self):
        a,z = deal(1,0,.01,commission=.1,fee=.02),deal(2,1,.01,commission=.03,fee=.01,swap=.2741,profit=-.1)
        self.assertAlmostEqual(closed_costs(z,[a,z])["net_profit"],.3341)

    def test_out_by(self):
        a,z=deal(1,0,.01,commission=-.07),deal(2,3,.01,profit=.5)
        self.assertAlmostEqual(closed_costs(z,[a,z])["net_profit"],.43)

    def test_exact_duplicates_do_not_double_cost(self):
        a,z=deal(1,0,.01,commission=-.07),deal(2,1,.01,profit=.5)
        self.assertAlmostEqual(closed_costs(z,[a,a,z,z])["net_profit"],.43)

    def test_foreign_position_excluded(self):
        a,z=deal(1,0,.01,commission=-.07),deal(2,1,.01)
        other=deal(3,0,999,ts=0,commission=-999)
        other["position_id"]=101
        self.assertAlmostEqual(closed_costs(z,[other,a,z])["entry_commission_alloc"],-.07)

    def assert_unresolved(self, z, h, reason):
        r=closed_costs(z,h)
        self.assertFalse(r["complete"])
        self.assertIsNone(r["net_profit"])
        self.assertEqual(r["incomplete_reason"],reason)

    def test_none_history_not_zero_fee(self):
        self.assert_unresolved(deal(2,1,.01,profit=.5),None,"position_history_unavailable")

    def test_missing_target(self):
        self.assert_unresolved(deal(2,1,.01),[deal(1,0,.01)],"target_exit_not_in_history")

    def test_missing_entry(self):
        z=deal(2,1,.01)
        self.assert_unresolved(z,[z],"exit_without_matching_entry")

    def test_missing_entry_fee(self):
        a,z=deal(1,0,.01),deal(2,1,.01)
        del a["fee"]
        self.assert_unresolved(z,[a,z],"missing_entry_cost_fields")

    def test_missing_exit_fee(self):
        a,z=deal(1,0,.01),deal(2,1,.01)
        del z["fee"]
        self.assert_unresolved(z,[a,z],"missing_exit_cost_fields")

    def test_oversized_exit(self):
        a,z=deal(1,0,.01),deal(2,1,.02)
        self.assert_unresolved(z,[a,z],"exit_exceeds_known_entry_volume")

    def test_inout_is_not_silently_hedging(self):
        a,rev,z=deal(1,0,.01),deal(2,2,.02),deal(3,1,.01)
        self.assert_unresolved(z,[a,rev,z],"inout_netting_requires_explicit_model")

    def test_zero_volume_cost_event_unresolved(self):
        a,c,z=deal(1,0,.01),deal(2,0,0,commission=-1),deal(3,1,.01)
        self.assert_unresolved(z,[a,c,z],"zero_or_invalid_volume_event")

    def test_conflicting_duplicate(self):
        a,z=deal(1,0,.01),deal(2,1,.01)
        changed=dict(a,commission=-1)
        self.assert_unresolved(z,[a,changed,z],"conflicting_duplicate_deal")

    def test_target_history_conflict(self):
        a,z=deal(1,0,.01),deal(2,1,.01)
        self.assert_unresolved(z,[a,dict(z,profit=1)],"target_exit_history_conflict")

    def test_nonfinite_costs_rejected(self):
        a,z=deal(1,0,.01),deal(2,1,.01,fee=math.nan)
        self.assert_unresolved(z,[a,z],"missing_exit_cost_fields")

    def test_finite_inputs_cannot_overflow_to_complete_net(self):
        a,z=deal(1,0,.01,commission=1e308),deal(2,1,.01,profit=1e308)
        r=closed_costs(z,[a,z])
        self.assert_unresolved(z,[a,z],"closed_net_overflow")
        self.assertIsNone(r["known_components_sum"])

    def test_entry_pool_overflow_is_explicitly_incomplete(self):
        a,b,z=deal(1,0,.01,commission=1e308),deal(2,0,.01,commission=1e308),deal(3,1,.02)
        self.assert_unresolved(z,[a,b,z],"entry_cost_pool_overflow")

    def test_boolean_position_id_is_not_valid_integer_identity(self):
        a,z=deal(1,0,.01),deal(2,1,.01)
        a["position_id"]=True
        z["position_id"]=True
        self.assert_unresolved(z,[a,z],"invalid_deal_identity")


if __name__ == "__main__":
    unittest.main()
