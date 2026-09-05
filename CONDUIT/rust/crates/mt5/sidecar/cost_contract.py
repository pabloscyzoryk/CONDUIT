"""Pure, causal closed-transche cost attribution; no MT5/network/filesystem calls.

This helper is not activated by the F13 release. The caller must provide the
complete position history and treat complete=False as unresolved, never net P&L.
Signed cost values are preserved; positive commissions/fees (rebates) are valid.
"""
import math
import hashlib
import json


def _get(row, key, default=None):
    return row.get(key, default) if isinstance(row, dict) else getattr(row, key, default)


def _number(row, key):
    value = _get(row, key)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    value = float(value)
    return value if math.isfinite(value) else None


def _key(row):
    ts, ticket = _get(row, "time_msc"), _get(row, "ticket")
    if isinstance(ts, bool) or isinstance(ticket, bool):
        return None
    if not isinstance(ts, int) or not isinstance(ticket, int) or ts < 0 or ticket <= 0:
        return None
    return ts, ticket


def closed_costs(deal, history):
    """Allocate entry commission/fee to one EXIT, with no future information.

    `history` must be full broker history for this position, including the exit.
    Allocation replays entry/out events chronologically through a residual pool;
    this is stateless across calls/restarts and does not count an entry fee again
    on later partials. OUT_BY is supported, INOUT/netting reversal is unresolved.
    Zero-volume cost-only events are unresolved until explicitly modelled.
    """
    result = {
        "schema": 1,
        "deal_id": _get(deal, "ticket"),
        "position_id": _get(deal, "position_id"),
        "gross_profit": _number(deal, "profit"),
        "entry_commission_alloc": None,
        "exit_commission": _number(deal, "commission"),
        "entry_fee_alloc": None,
        "exit_fee": _number(deal, "fee"),
        "swap": _number(deal, "swap"),
        "complete": False,
        "incomplete_reason": None,
        "net_profit": None,
    }

    def unresolved(reason):
        result["complete"] = False
        result["net_profit"] = None
        result["incomplete_reason"] = reason
        known = sum(
            result[k] for k in ("gross_profit", "entry_commission_alloc", "exit_commission", "entry_fee_alloc", "exit_fee", "swap")
            if result[k] is not None
        )
        result["known_components_sum"] = known if math.isfinite(known) else None
        return result

    target_key = _key(deal)
    position = _get(deal, "position_id")
    if target_key is None or type(position) is not int or position <= 0:
        return unresolved("invalid_deal_identity")
    if _get(deal, "entry") not in (1, 3):
        return unresolved("not_supported_exit")
    if any(result[k] is None for k in ("gross_profit", "exit_commission", "exit_fee", "swap")):
        return unresolved("missing_exit_cost_fields")
    if history is None:
        return unresolved("position_history_unavailable")
    seen = {}
    for row in history:
        if _get(row, "position_id") != position:
            continue
        key = _key(row)
        if key is None:
            return unresolved("invalid_history_identity")
        if key > target_key:
            continue
        identity = key[1]
        fingerprint = tuple(_get(row, k) for k in (
            "time_msc", "position_id", "entry", "type", "volume", "profit", "commission", "fee", "swap"))
        if identity in seen and seen[identity][0] != fingerprint:
            return unresolved("conflicting_duplicate_deal")
        seen[identity] = fingerprint, row
    if target_key[1] not in seen:
        return unresolved("target_exit_not_in_history")
    target_history = seen[target_key[1]][1]
    for name in ("time_msc", "entry", "type", "volume", "profit", "commission", "fee", "swap"):
        if _get(target_history, name) != _get(deal, name):
            return unresolved("target_exit_history_conflict")

    volume = 0.0
    commission_pool = 0.0
    fee_pool = 0.0
    direction = None
    for _, row in sorted(seen.values(), key=lambda item: _key(item[1])):
        entry = _get(row, "entry")
        side = _get(row, "type")
        vol = _number(row, "volume")
        if entry == 2:
            return unresolved("inout_netting_requires_explicit_model")
        if vol is None or vol <= 0.0:
            return unresolved("zero_or_invalid_volume_event")
        if side not in (0, 1):
            return unresolved("non_trade_cost_event_requires_model")
        if entry == 0:
            commission, fee = _number(row, "commission"), _number(row, "fee")
            if commission is None or fee is None:
                return unresolved("missing_entry_cost_fields")
            if direction is not None and side != direction:
                return unresolved("entry_direction_changed")
            direction = side
            volume += vol
            commission_pool += commission
            fee_pool += fee
            if not all(math.isfinite(x) for x in (volume, commission_pool, fee_pool)):
                return unresolved("entry_cost_pool_overflow")
        elif entry in (1, 3):
            if direction is None or side == direction:
                return unresolved("exit_without_matching_entry")
            tolerance = max(1e-10, volume * 1e-9)
            if vol > volume + tolerance:
                return unresolved("exit_exceeds_known_entry_volume")
            final = abs(vol - volume) <= tolerance
            fraction = 1.0 if final else vol / volume
            allocated_commission = commission_pool * fraction
            allocated_fee = fee_pool * fraction
            if _key(row) == target_key:
                result["entry_commission_alloc"] = allocated_commission
                result["entry_fee_alloc"] = allocated_fee
                result["net_profit"] = sum(result[k] for k in (
                    "gross_profit", "entry_commission_alloc", "exit_commission", "entry_fee_alloc", "exit_fee", "swap"))
                if not math.isfinite(result["net_profit"]):
                    return unresolved("closed_net_overflow")
                result["known_components_sum"] = result["net_profit"]
                result["complete"] = True
                return result
            volume = 0.0 if final else volume - vol
            commission_pool = 0.0 if final else commission_pool - allocated_commission
            fee_pool = 0.0 if final else fee_pool - allocated_fee
        else:
            return unresolved("unsupported_entry_kind")
    return unresolved("target_exit_not_processed")


def cost_receipt_payload(deal, history, currency):
    """Wire proof, not a second calculation of net or an account mutation.

    Only causal rows through (time_msc, deal ticket) enter the digest. Missing
    fields stay None. The adapter asserts successful full-position query; it
    cannot manufacture broker history that is unavailable or truncated.
    """
    calc = closed_costs(deal, history)
    target = _key(deal)
    fingerprint = None
    first_entry = None
    if calc["complete"]:
        rows = {}
        fields = ("ticket", "time_msc", "position_id", "entry", "type", "volume",
                  "profit", "commission", "fee", "swap")
        for row in history:
            key = _key(row)
            if _get(row, "position_id") == _get(deal, "position_id") and key is not None and key <= target:
                rows[key[1]] = [_get(row, name) for name in fields]
                if _get(row, "entry") == 0:
                    first_entry = key[0] if first_entry is None else min(first_entry, key[0])
        try:
            encoded = json.dumps(sorted(rows.values(), key=lambda r: (r[1], r[0])),
                                 ensure_ascii=True, separators=(",", ":"), allow_nan=False).encode("utf-8")
            fingerprint = hashlib.sha256(encoded).hexdigest()
        except (TypeError, ValueError):
            calc["complete"] = False
            calc["incomplete_reason"] = "invalid_history_fingerprint"
    if not isinstance(currency, str) or not currency.strip():
        calc["complete"] = False
        calc["incomplete_reason"] = "missing_account_currency"
    names = ("gross_profit", "entry_commission_alloc", "exit_commission",
             "entry_fee_alloc", "exit_fee", "swap")
    return dict(schema=1, deal_id=_get(deal, "ticket"), position_id=_get(deal, "position_id"),
                volume=_number(deal, "volume"), currency=currency if isinstance(currency,str) else "",
                complete=calc["complete"], incomplete_reason=calc["incomplete_reason"],
                cutoff_time_msc=_get(deal, "time_msc"), history_fingerprint=fingerprint,
                history_query_complete=history is not None, first_entry_time_msc=first_entry,
                **{name:calc[name] for name in names})
