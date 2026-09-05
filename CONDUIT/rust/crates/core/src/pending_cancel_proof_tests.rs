use super::*;

// Synthetic test model, NOT a captured MT5 cancellation. Native raw properties
// are a separate prerequisite before qualifying RemainingUnfilled in Bridge.
const SETUP: i64 = 1_700_000_000_000;
fn session() -> ExecutionSession {
    ExecutionSession {
        scope: "account-scope-demo".into(),
        generation: 7,
    }
}
fn binding(ticket: u64) -> PendingOrderBindingV1 {
    PendingOrderBindingV1 {
        order_ticket: ticket,
        symbol: "XAUUSD".into(),
        magic: 770077,
        kind: PendingKind::BuyLimit,
        level: 2,
        is_topup: false,
        is_toucher: false,
        time_setup_msc: SETUP,
        initial_units: 5,
        current_units_at_registration: 5,
        price: 4000.,
        sl: Some(3990.),
        tp: Some(4010.),
    }
}
fn intent() -> PendingCancelIntentV1 {
    PendingCancelIntentV1 {
        schema: PendingProofSchema::V1,
        key: PendingOperationKey {
            session: session(),
            owner_engine: "SYNERGY-slot-2".into(),
            basket_id: 21,
            operation_seq: 4,
        },
        revision: PendingRevision {
            source: 1,
            policy: 2,
            geometry: 3,
        },
        registered_after_observation: 40,
        registered_before_cancel_msc: SETUP + 1000,
        volume_step: 0.01,
        orders: vec![binding(700)],
    }
}
fn context(i: &PendingCancelIntentV1) -> PendingProofContext {
    PendingProofContext {
        session: Some(i.key.session.clone()),
        owner_engine: i.key.owner_engine.clone(),
        basket_id: i.key.basket_id,
        revision: i.revision,
        operation_still_current: true,
    }
}
fn history(b: &PendingOrderBindingV1) -> PendingOrderRecordV1 {
    PendingOrderRecordV1 {
        order_ticket: b.order_ticket,
        symbol: b.symbol.clone(),
        magic: b.magic,
        kind: b.kind,
        state: PendingOrderState::Canceled,
        time_setup_msc: b.time_setup_msc,
        time_done_msc: SETUP + 2000,
        position_identifier: 0,
        initial_units: b.initial_units,
        current_units: b.current_units_at_registration,
        price: b.price,
        sl: b.sl,
        tp: b.tp,
    }
}
fn observation(i: &PendingCancelIntentV1) -> PendingCancelObservationV1 {
    PendingCancelObservationV1 {
        schema: PendingProofSchema::V1,
        key: i.key.clone(),
        revision: i.revision,
        observation_seq: 41,
        history_read_through_msc: SETUP + 3000,
        identity_confirmed: true,
        session_before: session(),
        session_after: session(),
        locally_published_complete: true,
        volume_convention: HistoricalVolumeConventionV1::RemainingUnfilled {
            evidence_ref: "SYNTHETIC_remaining_unfilled_v1_NOT_NATIVE".into(),
        },
        current_orders: EvidenceRead::Complete(vec![]),
        orders: i
            .orders
            .iter()
            .map(|b| PendingOrderEvidenceV1 {
                queried_order_ticket: b.order_ticket,
                history: EvidenceRead::Complete(vec![history(b)]),
                deals: EvidenceRead::Complete(vec![]),
            })
            .collect(),
        related_positions: EvidenceRead::Complete(vec![]),
        receipt_cut: Some(PendingReceiptCutV1::Clear {
            required_owner_revision: 19,
            consumed_owner_revision: 19,
        }),
    }
}
fn run(i: PendingCancelIntentV1, o: PendingCancelObservationV1) -> PendingCancelStateV1 {
    let c = context(&i);
    PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession)
        .observe(&c, o)
        .clone()
}
fn hist(o: &mut PendingCancelObservationV1) -> &mut PendingOrderRecordV1 {
    match &mut o.orders[0].history {
        EvidenceRead::Complete(v) => &mut v[0],
        _ => panic!("fixture history"),
    }
}
fn deal(order: u64, ticket: u64) -> PendingDealV1 {
    PendingDealV1 {
        order_ticket: order,
        deal_ticket: ticket,
        position_identifier: 433,
        symbol: "XAUUSD".into(),
        magic: 770077,
        entry: 0,
        volume_units: 2,
        time_msc: SETUP + 1500,
    }
}
fn review(s: PendingCancelStateV1, r: PendingReviewReason) {
    assert_eq!(s, PendingCancelStateV1::RequiresReview(r));
}
fn waiting(s: PendingCancelStateV1) {
    assert!(matches!(s, PendingCancelStateV1::Waiting(_)), "{s:?}");
}

#[test]
fn synthetic_no_fill_complete_cancel_has_typed_nonzero_historical_remainder() {
    let i = intent();
    let o = observation(&i);
    let PendingCancelStateV1::VerifiedNoFill(p) = run(i, o) else {
        panic!("no proof")
    };
    assert_eq!(p.terminal_orders[0].history_remaining_units, 5);
    assert_eq!(p.retired_order_units, 5);
    assert_eq!(p.observation_seq, 41);
}
#[test]
fn unknown_cancel_volume_convention_never_uses_zero_or_initial_as_proof() {
    for current in [0, 5] {
        let i = intent();
        let mut o = observation(&i);
        hist(&mut o).current_units = current;
        o.volume_convention = HistoricalVolumeConventionV1::Unverified;
        review(run(i, o), PendingReviewReason::UnsupportedVolumeConvention);
    }
}
#[test]
fn missing_model_reference_is_not_provenance() {
    let i = intent();
    let mut o = observation(&i);
    o.volume_convention = HistoricalVolumeConventionV1::RemainingUnfilled {
        evidence_ref: " ".into(),
    };
    review(run(i, o), PendingReviewReason::UnsupportedVolumeConvention);
}
#[test]
fn absent_current_orders_without_final_history_is_only_waiting() {
    for history in [EvidenceRead::Complete(vec![]), EvidenceRead::Unavailable] {
        let i = intent();
        let mut o = observation(&i);
        o.orders[0].history = history;
        waiting(run(i, o));
    }
}
#[test]
fn cancel_ack_does_not_remove_still_current_order_from_proof() {
    let i = intent();
    let mut o = observation(&i);
    let mut p = history(&i.orders[0]);
    p.state = PendingOrderState::Placed;
    p.time_done_msc = 0;
    o.current_orders = EvidenceRead::Complete(vec![p]);
    waiting(run(i, o));
}

#[test]
fn filled_current_observation_is_sticky_review_before_later_canceled_history() {
    let i = intent();
    let c = context(&i);
    let mut first = observation(&i);
    let mut contradictory = history(&i.orders[0]);
    contradictory.state = PendingOrderState::Filled;
    // Deliberately inconsistent producer fields must not erase known Filled.
    assert_eq!(contradictory.position_identifier, 0);
    assert_eq!(contradictory.current_units, contradictory.initial_units);
    first.current_orders = EvidenceRead::Complete(vec![contradictory]);
    let mut later = observation(&i);
    later.observation_seq += 1;
    let mut r =
        PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
    review(
        r.observe(&c, first).clone(),
        PendingReviewReason::FillOrPartialObserved,
    );
    review(
        r.observe(&c, later).clone(),
        PendingReviewReason::FillOrPartialObserved,
    );
}
#[test]
fn started_placed_and_request_cancel_are_not_terminal() {
    for s in [
        PendingOrderState::Started,
        PendingOrderState::Placed,
        PendingOrderState::RequestAdd,
        PendingOrderState::RequestModify,
        PendingOrderState::RequestCancel,
    ] {
        let i = intent();
        let mut o = observation(&i);
        hist(&mut o).state = s;
        hist(&mut o).time_done_msc = 0;
        waiting(run(i, o));
    }
}
#[test]
fn terminal_expired_rejected_are_not_silently_client_cancel() {
    for s in [PendingOrderState::Expired, PendingOrderState::Rejected] {
        let i = intent();
        let mut o = observation(&i);
        hist(&mut o).state = s;
        review(run(i, o), PendingReviewReason::UnsupportedFinalState);
    }
}
#[test]
fn partial_filled_reduced_remainder_and_position_id_are_never_no_fill() {
    for case in 0..4 {
        let i = intent();
        let mut o = observation(&i);
        let h = hist(&mut o);
        match case {
            0 => h.state = PendingOrderState::Partial,
            1 => h.state = PendingOrderState::Filled,
            2 => h.current_units = 3,
            _ => h.position_identifier = 433,
        }
        review(run(i, o), PendingReviewReason::FillOrPartialObserved);
    }
}
#[test]
fn entry_deal_with_no_current_position_still_disproves_no_fill() {
    let i = intent();
    let mut o = observation(&i);
    o.orders[0].deals = EvidenceRead::Complete(vec![deal(700, 880)]);
    review(run(i, o), PendingReviewReason::FillOrPartialObserved);
}
#[test]
fn fill_then_close_and_zero_remaining_position_are_not_an_empty_lifecycle() {
    let i = intent();
    let mut o = observation(&i);
    o.related_positions = EvidenceRead::Complete(vec![RelatedPositionV1 {
        order_ticket: 700,
        position_identifier: 433,
        remaining_units: 0,
    }]);
    review(run(i, o), PendingReviewReason::FillOrPartialObserved);
}
#[test]
fn complete_read_errors_and_none_are_distinct_from_empty() {
    for field in 0..4 {
        for failed in [false, true] {
            let i = intent();
            let mut o = observation(&i);
            match field {
                0 => {
                    o.current_orders = if failed {
                        EvidenceRead::Failed
                    } else {
                        EvidenceRead::Unavailable
                    }
                }
                1 => {
                    o.related_positions = if failed {
                        EvidenceRead::Failed
                    } else {
                        EvidenceRead::Unavailable
                    }
                }
                2 => {
                    o.orders[0].history = if failed {
                        EvidenceRead::Failed
                    } else {
                        EvidenceRead::Unavailable
                    }
                }
                _ => {
                    o.orders[0].deals = if failed {
                        EvidenceRead::Failed
                    } else {
                        EvidenceRead::Unavailable
                    }
                }
            }
            let s = run(i, o);
            if failed {
                review(s, PendingReviewReason::ReadFailed)
            } else {
                waiting(s)
            }
        }
    }
}
#[test]
fn incomplete_atomic_local_publication_waits_even_with_plausible_individual_rows() {
    let i = intent();
    let mut o = observation(&i);
    o.locally_published_complete = false;
    waiting(run(i, o));
}
#[test]
fn duplicate_identical_history_is_idempotent_but_conflict_is_review() {
    for conflict in [false, true] {
        let i = intent();
        let mut o = observation(&i);
        let mut second = history(&i.orders[0]);
        if conflict {
            second.time_done_msc += 1;
        }
        if let EvidenceRead::Complete(rows) = &mut o.orders[0].history {
            rows.push(second);
        }
        let s = run(i, o);
        if conflict {
            review(s, PendingReviewReason::DuplicateConflict)
        } else {
            assert!(matches!(s, PendingCancelStateV1::VerifiedNoFill(_)));
        }
    }
}
#[test]
fn duplicate_conflicting_deal_is_review_not_double_counted() {
    let i = intent();
    let mut o = observation(&i);
    let a = deal(700, 880);
    let mut b = a.clone();
    b.volume_units = 3;
    o.orders[0].deals = EvidenceRead::Complete(vec![a, b]);
    review(run(i, o), PendingReviewReason::DuplicateConflict);
}
#[test]
fn multi_fill_order_in_any_deal_order_remains_unsupported() {
    for reverse in [false, true] {
        let i = intent();
        let mut o = observation(&i);
        let mut ds = vec![deal(700, 880), deal(700, 881)];
        if reverse {
            ds.reverse();
        }
        o.orders[0].deals = EvidenceRead::Complete(ds);
        review(run(i, o), PendingReviewReason::FillOrPartialObserved);
    }
}
#[test]
fn wrong_order_query_history_deal_symbol_or_magic_cannot_confirm() {
    for case in 0..5 {
        let i = intent();
        let mut o = observation(&i);
        match case {
            0 => o.orders[0].queried_order_ticket = 880,
            1 => hist(&mut o).order_ticket = 880,
            2 => hist(&mut o).symbol = "EURUSD".into(),
            3 => hist(&mut o).magic = 1,
            _ => o.orders[0].deals = EvidenceRead::Complete(vec![deal(701, 880)]),
        }
        review(run(i, o), PendingReviewReason::WrongOrderScope);
    }
}
#[test]
fn three_orders_same_level_including_topup_are_retired_as_one_exact_batch() {
    let mut i = intent();
    i.orders.push(binding(701));
    let mut topup = binding(702);
    topup.initial_units = 2;
    topup.current_units_at_registration = 2;
    topup.is_topup = true;
    i.orders.push(topup);
    let mut o = observation(&i);
    o.orders.reverse();
    let PendingCancelStateV1::VerifiedNoFill(p) = run(i, o) else {
        panic!("expected batch")
    };
    assert_eq!(p.retired_order_units, 12);
    assert_eq!(p.terminal_orders.len(), 3);
    assert_eq!(
        p.terminal_orders
            .iter()
            .map(|o| o.order_ticket)
            .collect::<Vec<_>>(),
        vec![700, 701, 702]
    );
}
#[test]
fn partial_cancel_success_cannot_publish_partial_batch_as_complete() {
    let mut i = intent();
    i.orders.push(binding(701));
    let mut o = observation(&i);
    let mut live = history(&i.orders[1]);
    live.state = PendingOrderState::Placed;
    live.time_done_msc = 0;
    o.current_orders = EvidenceRead::Complete(vec![live]);
    o.orders[1].history = EvidenceRead::Complete(vec![]);
    waiting(run(i.clone(), o));
    let mut missing = observation(&i);
    missing.orders.pop();
    waiting(run(i, missing));
}
#[test]
fn temporary_receipt_waits_and_unconsumed_revision_never_verifies() {
    for cut in [
        None,
        Some(PendingReceiptCutV1::Temporary),
        Some(PendingReceiptCutV1::Clear {
            required_owner_revision: 19,
            consumed_owner_revision: 18,
        }),
        Some(PendingReceiptCutV1::Clear {
            required_owner_revision: 0,
            consumed_owner_revision: 0,
        }),
    ] {
        let i = intent();
        let mut o = observation(&i);
        o.receipt_cut = cut;
        waiting(run(i, o));
    }
}
#[test]
fn receipt_review_is_sticky_even_if_next_snapshot_is_clear() {
    let i = intent();
    let c = context(&i);
    let mut o = observation(&i);
    let mut r =
        PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
    o.receipt_cut = Some(PendingReceiptCutV1::RequiresReview);
    review(
        r.observe(&c, o.clone()).clone(),
        PendingReviewReason::ReceiptReview,
    );
    o.observation_seq += 1;
    o.receipt_cut = Some(PendingReceiptCutV1::Clear {
        required_owner_revision: 19,
        consumed_owner_revision: 19,
    });
    review(r.observe(&c, o).clone(), PendingReviewReason::ReceiptReview);
}
#[test]
fn repeated_identical_observation_is_idempotent_and_does_not_add_quantities() {
    let i = intent();
    let c = context(&i);
    let o = observation(&i);
    let mut r =
        PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
    let first = r.observe(&c, o.clone()).clone();
    assert_eq!(r.observe(&c, o), &first);
}
#[test]
fn same_sequence_different_payload_or_older_sequence_is_review() {
    for older in [false, true] {
        let i = intent();
        let c = context(&i);
        let mut o = observation(&i);
        o.observation_seq = 43;
        let mut r =
            PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
        r.observe(&c, o.clone());
        if older {
            o.observation_seq = 42;
        } else {
            o.history_read_through_msc += 1;
        }
        review(
            r.observe(&c, o).clone(),
            if older {
                PendingReviewReason::StaleObservation
            } else {
                PendingReviewReason::ConflictingObservation
            },
        );
    }
}
#[test]
fn newer_complete_observation_can_finish_waiting_without_mutating_orders() {
    let i = intent();
    let c = context(&i);
    let mut first = observation(&i);
    first.orders[0].history = EvidenceRead::Unavailable;
    let mut second = observation(&i);
    second.observation_seq = 42;
    let mut r =
        PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
    waiting(r.observe(&c, first).clone());
    assert!(matches!(
        r.observe(&c, second),
        PendingCancelStateV1::VerifiedNoFill(_)
    ));
}
#[test]
fn account_a_b_a_old_generation_never_rebinds_old_proof() {
    let i = intent();
    let mut c = context(&i);
    let o = observation(&i);
    c.session.as_mut().unwrap().generation += 2;
    let mut r =
        PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
    review(
        r.observe(&c, o).clone(),
        PendingReviewReason::SessionMismatch,
    );
}
#[test]
fn session_change_between_reads_and_missing_current_session_fail_closed() {
    for case in 0..4 {
        let i = intent();
        let mut c = context(&i);
        let mut o = observation(&i);
        match case {
            0 => o.session_after.generation += 1,
            1 => o.session_before.scope = "other".into(),
            2 => o.identity_confirmed = false,
            _ => c.session = None,
        }
        let mut r =
            PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
        review(
            r.observe(&c, o).clone(),
            PendingReviewReason::SessionMismatch,
        );
    }
}
#[test]
fn source_policy_geometry_owner_and_cancelled_lifecycle_invalidate_old_intent() {
    for case in 0..6 {
        let i = intent();
        let mut c = context(&i);
        let o = observation(&i);
        let expected = match case {
            0 => {
                c.revision.source += 1;
                PendingReviewReason::RevisionMismatch
            }
            1 => {
                c.revision.policy += 1;
                PendingReviewReason::RevisionMismatch
            }
            2 => {
                c.revision.geometry += 1;
                PendingReviewReason::RevisionMismatch
            }
            3 => {
                c.owner_engine = "other".into();
                PendingReviewReason::OwnerMismatch
            }
            4 => {
                c.basket_id += 1;
                PendingReviewReason::OwnerMismatch
            }
            _ => {
                c.operation_still_current = false;
                PendingReviewReason::Superseded
            }
        };
        let mut r =
            PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
        review(r.observe(&c, o).clone(), expected);
    }
}
#[test]
fn cold_restored_dto_remains_review_even_with_new_plausible_observation() {
    let i = intent();
    let encoded = serde_json::to_string(&i).unwrap();
    let loaded: PendingCancelIntentV1 = serde_json::from_str(&encoded).unwrap();
    let mut r = PendingCancelReducerV1::new(loaded, PendingRegistrationOrigin::RestoredUnverified);
    review(
        r.observe(&context(&i), observation(&i)).clone(),
        PendingReviewReason::ColdRestart,
    );
}
#[test]
fn invalid_registration_missing_owner_duplicate_ticket_and_preexisting_partial_fail_closed() {
    for case in 0..7 {
        let mut i = intent();
        match case {
            0 => i.orders.clear(),
            1 => i.orders.push(i.orders[0].clone()),
            2 => i.orders[0].current_units_at_registration = 3,
            3 => i.key.owner_engine.clear(),
            4 => i.volume_step = f64::NAN,
            5 => i.orders[0].sl = Some(f64::NAN),
            _ => i.key.session.generation = 0,
        }
        let r =
            PendingCancelReducerV1::new(i, PendingRegistrationOrigin::RegisteredInCurrentSession);
        assert_eq!(
            r.state(),
            &PendingCancelStateV1::RequiresReview(PendingReviewReason::InvalidIntent)
        );
    }
}
#[test]
fn malformed_quantities_future_done_and_changed_geometry_do_not_verify() {
    for case in 0..6 {
        let i = intent();
        let mut o = observation(&i);
        let h = hist(&mut o);
        let expected = match case {
            0 => {
                h.current_units = 6;
                PendingReviewReason::QuantityConflict
            }
            1 => {
                h.initial_units = 6;
                PendingReviewReason::QuantityConflict
            }
            2 => {
                h.time_done_msc = SETUP + 4000;
                PendingReviewReason::MalformedEvidence
            }
            3 => {
                h.time_done_msc = SETUP + 500;
                PendingReviewReason::MalformedEvidence
            }
            4 => {
                h.price = 4001.;
                PendingReviewReason::GeometryConflict
            }
            _ => {
                h.sl = Some(f64::NAN);
                PendingReviewReason::GeometryConflict
            }
        };
        review(run(i, o), expected);
    }
}
#[test]
fn sum_overflow_is_review_not_wrapped_replacement_budget() {
    let mut i = intent();
    i.orders[0].initial_units = u64::MAX;
    i.orders[0].current_units_at_registration = u64::MAX;
    i.orders.push(binding(701));
    let o = observation(&i);
    review(run(i, o), PendingReviewReason::QuantityOverflow);
}
#[test]
fn serde_schema_is_required_versioned_and_unknown_fields_or_missing_units_fail() {
    let i = intent();
    let mut v = serde_json::to_value(&i).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["key"]["session"]["generation"], 7);
    v["schema"] = 2.into();
    assert!(serde_json::from_value::<PendingCancelIntentV1>(v).is_err());
    let mut v = serde_json::to_value(&i).unwrap();
    v.as_object_mut().unwrap().remove("schema");
    assert!(serde_json::from_value::<PendingCancelIntentV1>(v).is_err());
    let mut v = serde_json::to_value(observation(&i)).unwrap();
    v["unchecked_success"] = true.into();
    assert!(serde_json::from_value::<PendingCancelObservationV1>(v).is_err());
    let mut v = serde_json::to_value(history(&i.orders[0])).unwrap();
    v.as_object_mut().unwrap().remove("current_units");
    assert!(serde_json::from_value::<PendingOrderRecordV1>(v).is_err());
}
#[test]
fn mt5_state_codes_are_explicit_and_unknown_cannot_become_canceled() {
    assert_eq!(
        PendingOrderState::from_mt5(2),
        Some(PendingOrderState::Canceled)
    );
    assert_eq!(
        PendingOrderState::from_mt5(3),
        Some(PendingOrderState::Partial)
    );
    assert_eq!(
        PendingOrderState::from_mt5(9),
        Some(PendingOrderState::RequestCancel)
    );
    assert_eq!(PendingOrderState::from_mt5(10), None);
    assert_eq!(PendingOrderState::from_mt5(-1), None);
}
