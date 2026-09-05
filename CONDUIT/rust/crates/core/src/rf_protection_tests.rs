use super::*;

fn owner() -> RfOwner {
    RfOwner {
        scope_id: "existing-adapter-scope:DEMO-42/XAUUSD".into(),
        session_generation: 7,
        basket_id: 9,
        setup_revision: 3,
        policy_revision: 4,
    }
}
fn zero_costs() -> RfResidualCosts {
    RfResidualCosts {
        entry_commission: Some(0.),
        entry_fee: Some(0.),
        accrued_swap: Some(0.),
        modelled_exit_commission: Some(0.),
        modelled_exit_fee: Some(0.),
    }
}
// Only the fixture uses XAU100/USD. The production evaluator accepts explicit
// already-valued gross profit; it does not assume a contract or FX conversion.
fn input(realized: f64, stop: f64) -> RfFloorInput {
    let model = "fixture:XAU100-USD/no-future-fees/v1".to_string();
    RfFloorInput {
        schema: RfFloorSchema::V1,
        expected_owner: owner(),
        currency: "USD".into(),
        valuation_model_id: model.clone(),
        minimum_snapshot_revision: 12,
        required_ledger_revision: 5,
        snapshot: RfSnapshot {
            owner: owner(),
            revision: 12,
            authoritative: true,
            all_owned_positions_complete: true,
            positions: vec![RfPosition {
                owner: owner(),
                position_identifier: 900_001,
                remaining_volume: 0.05,
                stop: RfStopEvidence::Confirmed {
                    price: stop,
                    snapshot_revision: 12,
                },
            }],
            pending: RfPendingEvidence::NoneConfirmed {
                snapshot_revision: 12,
                cancellation_and_fills_reconciled: true,
            },
        },
        realized: RfRealized {
            owner: owner(),
            ledger_revision: 5,
            currency: "USD".into(),
            profit_basis: ProfitBasis::CanonicalClosedNetV1,
            receipts_complete_and_consumed: true,
            net: Some(realized),
        },
        legs: vec![RfLegValuation {
            owner: owner(),
            position_identifier: 900_001,
            snapshot_revision: 12,
            ledger_revision: 5,
            currency: "USD".into(),
            model_id: model,
            for_volume: 0.05,
            at_stop_price: stop,
            allocation_complete: true,
            gross_profit_at_stop: Some((stop - 4436.57) * 0.05 * 100.),
            costs: zero_costs(),
        }],
    }
}
fn verified(input: &RfFloorInput) -> RfNominalFloor {
    match evaluate_nominal_floor(input).outcome {
        RfFloorOutcome::VerifiedNominal { calculation } => calculation,
        other => panic!("expected verified: {other:?}"),
    }
}
fn incomplete(input: &RfFloorInput) -> Vec<RfIncomplete> {
    match evaluate_nominal_floor(input).outcome {
        RfFloorOutcome::Incomplete { issues } => issues,
        other => panic!("expected incomplete: {other:?}"),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}

#[test]
fn b9_closed_loss_plus_confirmed_own_be_is_unsecured_not_zero() {
    match evaluate_nominal_floor(&input(-42.65, 4436.57)).outcome {
        RfFloorOutcome::Unsecured {
            calculation: Some(c),
            reasons,
        } => {
            near(c.nominal_net, -42.65);
            near(c.remaining_gross_at_stops, 0.);
            assert_eq!(reasons, vec![RfUnsecured::NegativeNominalFloor]);
        }
        other => panic!("B9 own BE must not erase banked loss: {other:?}"),
    }
}
#[test]
fn profitable_confirmed_nominal_floor_includes_signed_costs() {
    let mut i = input(12., 4437.57);
    i.legs[0].costs = RfResidualCosts {
        entry_commission: Some(-0.2),
        entry_fee: Some(-0.03),
        accrued_swap: Some(-0.4),
        modelled_exit_commission: Some(-0.15),
        modelled_exit_fee: Some(-0.02),
    };
    let c = verified(&i);
    near(c.remaining_gross_at_stops, 5.);
    near(c.remaining_signed_costs, -0.8);
    near(c.nominal_net, 16.2);
}
#[test]
fn stop_off_is_known_unsecured_without_a_fabricated_numeric_floor() {
    let mut i = input(50., 4436.57);
    i.snapshot.positions[0].stop = RfStopEvidence::Absent;
    i.legs.clear();
    assert!(
        matches!(evaluate_nominal_floor(&i).outcome,RfFloorOutcome::Unsecured{calculation:None,reasons}
        if reasons==vec![RfUnsecured::NoBrokerStop{position_identifier:900_001}])
    );
}
#[test]
fn rejected_or_only_requested_sl_is_not_a_confirmation() {
    let mut i = input(50., 4437.57);
    i.snapshot.positions[0].stop = RfStopEvidence::Unconfirmed;
    assert!(incomplete(&i).contains(&RfIncomplete::StopUnconfirmed {
        position_identifier: 900_001
    }));
}
#[test]
fn accepted_rpc_optimistic_cache_is_not_authoritative() {
    let mut i = input(50., 4437.57);
    i.snapshot.authoritative = false;
    assert!(incomplete(&i).contains(&RfIncomplete::SnapshotNotAuthoritative));
}
#[test]
fn stale_snapshot_or_stop_revision_cannot_verify() {
    let mut i = input(50., 4437.57);
    i.minimum_snapshot_revision = 13;
    assert!(incomplete(&i).contains(&RfIncomplete::SnapshotStale));
    i.minimum_snapshot_revision = 12;
    i.snapshot.positions[0].stop = RfStopEvidence::Confirmed {
        price: 4437.57,
        snapshot_revision: 11,
    };
    assert!(incomplete(&i).contains(&RfIncomplete::StopStaleOrInvalid {
        position_identifier: 900_001
    }));
}
#[test]
fn unknown_cost_is_incomplete_not_assumed_zero() {
    for field in 0..5 {
        let mut i = input(50., 4437.57);
        let costs = &mut i.legs[0].costs;
        match field {
            0 => costs.entry_commission = None,
            1 => costs.entry_fee = None,
            2 => costs.accrued_swap = None,
            3 => costs.modelled_exit_commission = None,
            _ => costs.modelled_exit_fee = None,
        };
        assert!(
            incomplete(&i).contains(&RfIncomplete::ValuationOrCostIncomplete {
                position_identifier: 900_001
            })
        );
    }
}
#[test]
fn existing_tighter_confirmed_stop_needs_no_fresh_modify() {
    let c = verified(&input(-4., 4438.57));
    near(c.nominal_net, 6.);
    // No requested-stop/armed-count input exists. The existing actual SL is sufficient.
}
#[test]
fn partial_volume_cannot_reuse_full_volume_valuation() {
    let mut i = input(0., 4437.57);
    i.legs[0].for_volume = 0.30;
    assert!(
        incomplete(&i).contains(&RfIncomplete::ValuationBindingMismatch {
            position_identifier: 900_001
        })
    );
    i.legs[0].for_volume = 0.05;
    near(verified(&i).nominal_net, 5.);
}
#[test]
fn partial_cost_residual_is_not_charged_twice() {
    // Closed .25 already includes -2.50 entry commission. Remaining .05 owes
    // only -.50; realized net is supplied, never recounted by the evaluator.
    let mut i = input(7.5, 4437.57);
    i.legs[0].costs.entry_commission = Some(-0.5);
    near(verified(&i).nominal_net, 12.0);
    i.legs[0].allocation_complete = false;
    assert!(!incomplete(&i).is_empty());
}
#[test]
fn owner_account_generation_setup_and_policy_are_bound() {
    for field in 0..5 {
        let mut i = input(50., 4437.57);
        let o = &mut i.snapshot.positions[0].owner;
        match field {
            0 => o.scope_id = "another account".into(),
            1 => o.session_generation += 1,
            2 => o.basket_id += 1,
            3 => o.setup_revision += 1,
            _ => o.policy_revision += 1,
        };
        assert!(incomplete(&i).contains(&RfIncomplete::OwnerMismatch));
    }
}
#[test]
fn realized_requires_owned_complete_canonical_net_at_required_revision() {
    let mut i = input(50., 4437.57);
    i.realized.net = None;
    assert!(incomplete(&i).contains(&RfIncomplete::RealizedMissingOrNonfinite));
    i.realized.net = Some(50.);
    i.realized.receipts_complete_and_consumed = false;
    assert!(incomplete(&i).contains(&RfIncomplete::RealizedNotCanonicalOrUnconsumed));
    i.realized.receipts_complete_and_consumed = true;
    i.realized.profit_basis = ProfitBasis::LegacySourceDefined;
    assert!(incomplete(&i).contains(&RfIncomplete::RealizedNotCanonicalOrUnconsumed));
    i.realized.profit_basis = ProfitBasis::CanonicalClosedNetV1;
    i.realized.ledger_revision = 4;
    assert!(incomplete(&i).contains(&RfIncomplete::RealizedRevisionMismatch));
    i.realized.ledger_revision = 5;
    i.realized.owner.basket_id = 10;
    assert!(incomplete(&i).contains(&RfIncomplete::OwnerMismatch));
}
#[test]
fn currency_and_valuation_model_cannot_be_mixed() {
    let mut i = input(50., 4437.57);
    i.legs[0].currency = "EUR".into();
    assert!(incomplete(&i).contains(&RfIncomplete::CurrencyMismatch));
    i.legs[0].currency = "USD".into();
    i.legs[0].model_id = "different-contract".into();
    assert!(
        incomplete(&i).contains(&RfIncomplete::ValuationBindingMismatch {
            position_identifier: 900_001
        })
    );
}
#[test]
fn pending_cancel_ack_or_empty_unreconciled_snapshot_is_incomplete() {
    let mut i = input(50., 4437.57);
    i.snapshot.pending = RfPendingEvidence::Unknown;
    assert!(incomplete(&i).contains(&RfIncomplete::PendingUnconfirmed));
    i.snapshot.pending = RfPendingEvidence::NoneConfirmed {
        snapshot_revision: 12,
        cancellation_and_fills_reconciled: false,
    };
    assert!(incomplete(&i).contains(&RfIncomplete::PendingUnconfirmed));
    i.snapshot.pending = RfPendingEvidence::NoneConfirmed {
        snapshot_revision: 11,
        cancellation_and_fills_reconciled: true,
    };
    assert!(incomplete(&i).contains(&RfIncomplete::PendingUnconfirmed));
}
#[test]
fn known_pending_risk_is_unsecured_not_ignored() {
    let mut i = input(50., 4437.57);
    i.snapshot.pending = RfPendingEvidence::Present;
    assert!(
        matches!(evaluate_nominal_floor(&i).outcome,RfFloorOutcome::Unsecured{calculation:None,reasons}
        if reasons==vec![RfUnsecured::PendingExposureNotValued])
    );
}
#[test]
fn missing_duplicate_or_extra_position_valuation_is_not_a_valid_total() {
    let mut i = input(50., 4437.57);
    i.legs.clear();
    assert!(incomplete(&i).contains(&RfIncomplete::MissingValuation {
        position_identifier: 900_001
    }));
    i = input(50., 4437.57);
    i.legs.push(i.legs[0].clone());
    assert!(incomplete(&i).contains(&RfIncomplete::DuplicateValuation {
        position_identifier: 900_001
    }));
    i = input(50., 4437.57);
    i.legs[0].position_identifier = 8;
    assert!(incomplete(&i).contains(&RfIncomplete::UnexpectedValuation {
        position_identifier: 8
    }));
    i = input(50., 4437.57);
    i.snapshot.positions.push(i.snapshot.positions[0].clone());
    assert!(incomplete(&i).contains(&RfIncomplete::DuplicatePosition {
        position_identifier: 900_001
    }));
    i = input(50., 4437.57);
    i.snapshot.all_owned_positions_complete = false;
    assert!(incomplete(&i).contains(&RfIncomplete::PositionInventoryIncomplete));
}
#[test]
fn nonfinite_and_invalid_values_fail_closed() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut i = input(1., 4437.57);
        i.realized.net = Some(value);
        assert!(!incomplete(&i).is_empty());
        i = input(1., 4437.57);
        i.legs[0].costs.accrued_swap = Some(value);
        assert!(!incomplete(&i).is_empty());
        i = input(1., 4437.57);
        i.legs[0].gross_profit_at_stop = Some(value);
        assert!(!incomplete(&i).is_empty());
    }
    for volume in [0., -0.01, f64::NAN, f64::INFINITY] {
        let mut i = input(1., 4437.57);
        i.snapshot.positions[0].remaining_volume = volume;
        assert!(!incomplete(&i).is_empty());
    }
}
#[test]
fn checked_sum_overflow_is_incomplete() {
    let mut i = input(f64::MAX, 4437.57);
    i.legs[0].gross_profit_at_stop = Some(f64::MAX);
    assert_eq!(incomplete(&i), vec![RfIncomplete::ArithmeticOverflow]);
}
#[test]
fn zero_verified_but_tiny_negative_is_not_rounded_away() {
    near(verified(&input(0., 4436.57)).nominal_net, 0.);
    assert!(matches!(
        evaluate_nominal_floor(&input(-1e-12, 4436.57)).outcome,
        RfFloorOutcome::Unsecured {
            calculation: Some(_),
            ..
        }
    ));
}
#[test]
fn flat_owned_complete_account_uses_actual_realized_not_an_invented_zero() {
    let mut i = input(3., 4436.57);
    i.snapshot.positions.clear();
    i.legs.clear();
    near(verified(&i).nominal_net, 3.);
    i.realized.net = None;
    assert!(!incomplete(&i).is_empty());
}
#[test]
fn explicit_generic_valuation_has_no_hidden_xau_or_usd_multiplier() {
    let mut i = input(2., 4437.57);
    i.currency = "EUR".into();
    i.realized.currency = "EUR".into();
    i.legs[0].currency = "EUR".into();
    i.valuation_model_id = "explicit-other-contract-FX-v1".into();
    i.legs[0].model_id = i.valuation_model_id.clone();
    i.legs[0].gross_profit_at_stop = Some(7.25);
    near(verified(&i).nominal_net, 9.25);
}
#[test]
fn schema_roundtrip_unknown_version_and_missing_required_fields() {
    let i = input(-42.65, 4436.57);
    let value = serde_json::to_value(&i).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(
        serde_json::from_value::<RfFloorInput>(value.clone()).unwrap(),
        i
    );
    let r = evaluate_nominal_floor(&i);
    assert_eq!(
        serde_json::from_value::<RfFloorResult>(serde_json::to_value(&r).unwrap()).unwrap(),
        r
    );
    let mut bad = value.clone();
    bad["schema"] = serde_json::json!(2);
    assert!(serde_json::from_value::<RfFloorInput>(bad).is_err());
    let mut bad = value;
    bad["snapshot"]
        .as_object_mut()
        .unwrap()
        .remove("authoritative");
    assert!(serde_json::from_value::<RfFloorInput>(bad).is_err());
}
#[test]
fn input_order_does_not_change_the_float_sum() {
    let mut i = input(2., 4437.57);
    for id in [900_002, 900_003] {
        let mut p = i.snapshot.positions[0].clone();
        p.position_identifier = id;
        i.snapshot.positions.push(p);
        let mut l = i.legs[0].clone();
        l.position_identifier = id;
        l.gross_profit_at_stop = Some(if id == 900_002 { 1e6 } else { -1e6 });
        i.legs.push(l);
    }
    let before = evaluate_nominal_floor(&i);
    i.legs.reverse();
    i.snapshot.positions.reverse();
    assert_eq!(evaluate_nominal_floor(&i), before);
}
#[test]
fn a_future_gap_can_be_worse_than_the_verified_nominal_floor() {
    let c = verified(&input(0., 4437.57));
    near(c.nominal_net, 5.);
    let hypothetical_future_gap_fill = 4433.57;
    let actual_future_profit = (hypothetical_future_gap_fill - 4436.57) * 0.05 * 100.;
    assert!(actual_future_profit < c.nominal_net);
    // Future execution is deliberately absent from evaluator inputs. Calling
    // its result a gap-safe lower bound would contradict this accepted test.
}
