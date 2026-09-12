"""Synthetic mapping checks; no terminal, network or trading API."""
from pathlib import Path
import unittest

import contract
from mapping import MAP, ENUMY


class LotGrowthMappingTests(unittest.TestCase):
    def baseline(self):
        settings = {name: False if typ is bool else "" if typ is str else 0
                    for name, _, typ in MAP}
        settings.update({name: next(iter(values)) for name, (_, values) in ENUMY.items()})
        settings.update(contract.LOT_GROWTH_DEFAULTS)
        source = (Path(__file__).resolve().parents[1] / "mql5/CONDUIT_XT.mq5").read_text(encoding="utf-8")
        return settings, source

    def test_older_binary_has_explicit_off_not_inferred_active_defaults(self):
        settings, source = self.baseline()
        settings = {k: v for k, v in settings.items() if not k.startswith("lot_growth_")}
        inputs, report = contract.build(settings, {}, source, "synthetic.csv")
        self.assertEqual(report["errors"], [])
        self.assertEqual(inputs["In_LotGrowthMode"], 0)
        self.assertEqual(report["fields"]["lot_growth_mode"], "older_binary_feature_absent_disabled_in_ea")
        _, partial = contract.build(settings, {"lot_growth_mode": "Power"}, source, "synthetic.csv")
        self.assertFalse(partial["mapping_complete"])
        self.assertIn("lot_growth_reference_lot", [e["field"] for e in partial["errors"]])

    def test_each_mode_preserves_exact_supplied_parameters(self):
        settings, source = self.baseline()
        supplied = {"lot_growth_reference_lot": 0.015, "lot_growth_reference_balance": 350.0,
                    "lot_growth_power": 0.6, "lot_growth_rate_pct": 0.45,
                    "lot_growth_capital_multiple": 3.0, "lot_growth_lot_multiple": 1.7,
                    "lot_growth_basket_risk_pct": 8.0}
        for mode, enum in [("Off", 0), ("Power", 1), ("ThresholdLinear", 2), ("GeometricSteps", 3)]:
            inputs, report = contract.build(settings, {**supplied, "lot_growth_mode": mode}, source, "synthetic.csv")
            self.assertEqual(report["errors"], [])
            self.assertEqual(inputs["In_LotGrowthMode"], enum)
            for name, target, _ in MAP:
                if name in supplied:
                    self.assertEqual(inputs[target], supplied[name])
            self.assertFalse(report["execution_parity_proven"])

    def test_missing_native_input_and_invalid_numeric_type_cannot_qualify(self):
        settings, source = self.baseline()
        source = source.replace("In_LotGrowthPower", "RenamedUnsupportedInput")
        _, missing = contract.build(settings, {}, source, "synthetic.csv")
        self.assertIn("lot_growth_power", [e["field"] for e in missing["errors"]])
        settings, source = self.baseline()
        for bad in [True, "0.7", float("nan"), float("inf")]:
            _, report = contract.build(settings, {"lot_growth_power": bad}, source, "synthetic.csv")
            self.assertIn({"field": "lot_growth_power", "reason": "finite_numeric_type_required"}, report["errors"])

    def test_unsupported_autonomous_policy_is_still_rejected(self):
        settings, source = self.baseline()
        for active in [{"ea_enabled": True}, {"t100": {"enabled": True}}]:
            _, report = contract.build(settings, {"lot_growth_mode": "Power", **active}, source, "synthetic.csv")
            self.assertFalse(report["mapping_complete"])
            self.assertTrue(set(active) & {e["field"] for e in report["errors"]})

    def test_all_allocations_and_ten_strengths_map_without_changing_g7_fields(self):
        settings, source = self.baseline()
        inputs_off, _ = contract.build(settings, {}, source, "synthetic.csv")
        strengths = {k: (i + 1) / 10 for i, k in enumerate(
            k for k in contract.LOT_GROWTH_DEFAULTS if k.endswith("_strength"))}
        self.assertEqual(len(strengths), 10)
        for allocation, enum in ENUMY["lot_growth_allocation"][1].items():
            inputs, report = contract.build(settings, {"lot_growth_mode": "Power",
                "lot_growth_allocation": allocation, **strengths}, source, "synthetic.csv")
            self.assertEqual(report["errors"], [])
            self.assertEqual(inputs["In_LotGrowthAllocation"], enum)
            for name, value in inputs_off.items():
                if not name.startswith("In_LotGrowth"):
                    self.assertEqual(inputs[name], value, name)
            for name, target, _ in MAP:
                if name in strengths:
                    self.assertEqual(inputs[target], strengths[name])

    def test_active_parameter_domains_and_off_ignoring_inactive_values(self):
        settings, source = self.baseline()
        cases = [("Power", "lot_growth_reference_lot", .009),
                 ("Power", "lot_growth_reference_balance", 0),
                 ("Power", "lot_growth_power", 1.01),
                 ("ThresholdLinear", "lot_growth_rate_pct", -.1),
                 ("GeometricSteps", "lot_growth_capital_multiple", 1),
                 ("GeometricSteps", "lot_growth_lot_multiple", .9),
                 ("Power", "lot_growth_basket_risk_pct", 100.01)]
        cases += [("Power", name, value) for name in contract.LOT_GROWTH_DEFAULTS
                  if name.endswith("_strength") for value in [-.1, 2.01]]
        for mode, field, bad in cases:
            _, report = contract.build(settings, {"lot_growth_mode": mode, field: bad}, source, "synthetic.csv")
            self.assertIn({"field": field, "reason": "invalid_active_lot_growth_domain"}, report["errors"])
            _, inactive = contract.build(settings, {"lot_growth_mode": "Off", field: bad}, source, "synthetic.csv")
            self.assertEqual(inactive["errors"], [])
        for field in ["lot_growth_mode", "lot_growth_allocation"]:
            for bad in [[], {}, "unknown"]:
                _, report = contract.build(settings, {field: bad}, source, "synthetic.csv")
                self.assertIn({"field": field, "reason": "missing_or_unsupported_enum"}, report["errors"])

    def test_reconciled_relot_remains_explicitly_unsupported_in_native(self):
        settings, source = self.baseline()
        _, report = contract.build(settings, {"lot_growth_mode": "Power",
            "pending_relot_reconcile_target": True}, source, "synthetic.csv")
        self.assertIn({"field": "pending_relot_reconcile_target", "reason": "unsupported_active_or_unclassified_setting"}, report["errors"])

    def test_native_sizing_scenarios_are_gated_and_all_production_sends_are_wrapped(self):
        _, source = self.baseline()
        self.assertIn("In_TestExitScenario > 27", source)
        for scenario, function in [(22, "LotGrowthMathScenarioTick"),
                                   (23, "LotGrowthSendScenarioTick"),
                                   (24, "LotGrowthStressScenarioTick"),
                                   (27, "LotGrowthPendingPriceScenarioTick")]:
            self.assertIn(f"if(In_TestExitScenario == {scenario}) {{ {function}(); return; }}", source)
        # This is a call-boundary structural guard, not a native execution test.
        for name, ending in [("bool WyslijRynek(", "bool WyslijLimit("),
                             ("bool WyslijLimit(", "// budżet ryzyka wejść")]:
            body = source.split(name, 1)[1].split(ending, 1)[0]
            sequence = ["GrowthReady(bi)", "GrowthAllocate(", "ProfitBudgetFloorVolume(",
                        "ProfitBudgetLimit(bi,g_b", "GrowthBudgetLimit(", "OrderSend(r, res)"]
            positions = [body.index(item) for item in sequence]
            self.assertEqual(positions, sorted(positions))
            self.assertIn("GrowthRememberUnknown(bi,r,res)", body)
        pending = source.split("bool WyslijLimit(", 1)[1].split("// budżet ryzyka wejść", 1)[0]
        sequence = ["NormPx(price)", "if(GrowthEnabled() && !PendingPxIsValid(typ,r.price))",
                    "GrowthAllocate(", "AuditOpenRequest(", "OrderSend(r, res)"]
        self.assertEqual([pending.index(x) for x in sequence], sorted(pending.index(x) for x in sequence))
        self.assertIn("factor==1.0 || requested<=minimum ? requested", source)
        self.assertIn("double per_order=cel_szczebla/(double)sztuki", source)
        self.assertIn("cel_szczebla=per_order*(double)sztuki", source)


if __name__ == "__main__":
    unittest.main()
