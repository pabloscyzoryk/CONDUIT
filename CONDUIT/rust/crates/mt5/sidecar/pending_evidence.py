"""Read-only pending-history DTO producer; NOT imported by mt5_sidecar.

No MetaTrader5 import, connection, subscription, mutation, clock, filesystem,
network, seen-deal set or consumer ledger. The caller supplies an already-bound
read API. Every returned list is retained, including invalid/duplicate records.
An operation token is only an echo, not proof of app transport generation.
The API cannot prove an atomic history cutoff or cancellation volume semantics:
both remain explicitly unverified, even when all raw reads are successful.
"""
from __future__ import annotations

import math
from collections.abc import Mapping


SCHEMA = "conduit.pending-evidence.raw.v1"


def _positive_id(value):
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def _nonnegative_int(value):
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def _finite_nonnegative(value):
    try:
        return (isinstance(value, (int, float)) and not isinstance(value, bool)
                and math.isfinite(value) and value >= 0)
    except OverflowError:
        return False


def _raw(value):
    """Lossless for ordinary MT5 scalar values; explicit tags for invalid data."""
    if value is None or isinstance(value, (str, bool, int)):
        return value
    if isinstance(value, float):
        return value if math.isfinite(value) else {"_non_finite": repr(value)}
    if isinstance(value, Mapping):
        return {str(k): _raw(v) for k, v in value.items()}
    if isinstance(value, (tuple, list)):
        return [_raw(v) for v in value]
    return {"_unsupported_type": type(value).__name__}


def _record(value):
    if isinstance(value, Mapping):
        return dict(value)
    if hasattr(value, "_asdict"):
        return dict(value._asdict())
    if hasattr(value, "__dict__"):
        return dict(vars(value))
    raise ValueError("record is not a named tuple or mapping")


def _tagged_invalid(value):
    if isinstance(value, dict):
        return ("_non_finite" in value or "_unsupported_type" in value
                or any(_tagged_invalid(v) for v in value.values()))
    if isinstance(value, list):
        return any(_tagged_invalid(v) for v in value)
    return False


def _identity(value):
    try:
        v = _record(value)
    except (TypeError, ValueError):
        return None
    if (not _positive_id(v.get("login")) or not isinstance(v.get("server"), str)
            or not v["server"].strip() or not _nonnegative_int(v.get("trade_mode"))
            or v["trade_mode"] not in (0, 1, 2)):
        return None
    return {"login": v["login"], "server": v["server"], "trade_mode": v["trade_mode"]}


def _account(api):
    try:
        value = api.account_info()
        if value is None:
            return {"status": "none", "identity": None}
        ident = _identity(value)
        return {"status": "complete" if ident else "invalid", "identity": ident}
    except Exception as exc:
        return {"status": "error", "identity": None, "error_type": type(exc).__name__}


_ORDER_INTS = ("type", "state", "magic", "position_id", "position_by_id",
               "time_setup_msc", "time_done_msc")
_ORDER_NUMBERS = ("volume_initial", "volume_current", "price_open", "sl", "tp")
_DEAL_INTS = ("order", "type", "entry", "magic", "position_id", "time_msc")
_DEAL_NUMBERS = ("volume", "price")
_POSITION_INTS = ("identifier", "type", "magic", "time_msc")
_POSITION_NUMBERS = ("volume", "price_open", "sl", "tp")


def _validate_record(row, kind, binding):
    errors = []
    if not _positive_id(row.get("ticket")):
        errors.append("ticket")
    if not isinstance(row.get("symbol"), str) or not row["symbol"].strip():
        errors.append("symbol")
    ints, numbers = {"order": (_ORDER_INTS, _ORDER_NUMBERS),
                     "deal": (_DEAL_INTS, _DEAL_NUMBERS),
                     "position": (_POSITION_INTS, _POSITION_NUMBERS)}[kind]
    errors.extend(key for key in ints if not _nonnegative_int(row.get(key)))
    errors.extend(key for key in numbers if not _finite_nonnegative(row.get(key)))
    if binding is not None:
        expected = binding["order_ticket"]
        order_value = row.get("order") if kind == "deal" else row.get("ticket")
        if order_value != expected:
            errors.append("queried_order_mismatch")
        if row.get("symbol") != binding["symbol"] or row.get("magic") != binding["magic"]:
            errors.append("owner_mismatch")
    return errors


def _read(fn, kind, binding=None, **kwargs):
    try:
        values = fn(**kwargs)
    except Exception as exc:
        return {"status": "error", "records": None, "error_type": type(exc).__name__}
    if values is None:
        return {"status": "none", "records": None}
    if not isinstance(values, (tuple, list)):
        return {"status": "invalid", "records": _raw(values), "errors": ["list_required"]}
    records, errors, duplicates, seen = [], [], [], {}
    for index, value in enumerate(values):
        try:
            row = _record(value)
        except (TypeError, ValueError) as exc:
            records.append(_raw(value))
            errors.append({"index": index, "fields": [str(exc)]})
            continue
        records.append(_raw(row))
        invalid = _validate_record(row, kind, binding)
        if _tagged_invalid(records[-1]):
            invalid.append("non_json_value_preserved_as_diagnostic_tag")
        if invalid:
            errors.append({"index": index, "fields": invalid})
        ticket = row.get("ticket")
        if _positive_id(ticket):
            if ticket in seen:
                if seen[ticket] != row:
                    errors.append({"index": index, "fields": ["conflicting_duplicate"]})
                else:
                    duplicates.append(ticket)
            seen[ticket] = row
    result = {"status": "invalid" if errors else "complete", "records": records}
    if errors:
        result["errors"] = errors
    if duplicates:
        result["identical_duplicate_tickets"] = duplicates
    return result


def _bindings(values):
    if not isinstance(values, (list, tuple)) or not values:
        raise ValueError("nonempty registered order bindings required")
    out, seen = [], set()
    for raw in values:
        b = _record(raw)
        if (set(b) != {"order_ticket", "symbol", "magic"}
                or not _positive_id(b.get("order_ticket")) or b["order_ticket"] in seen
                or not isinstance(b.get("symbol"), str) or not b["symbol"].strip()
                or not _positive_id(b.get("magic"))):
            raise ValueError("invalid, duplicate or incomplete order binding")
        seen.add(b["order_ticket"])
        out.append(b)
    return out


def _links(order_reads, positions):
    """Observed entry-deal relationships only. Never ticket == identifier."""
    out = []
    for item in order_reads:
        order = item["binding"]["order_ticket"]
        ds, hs = item["deals"], item["history"]
        if ds["status"] != "complete" or positions["status"] != "complete":
            out.append({"order_ticket": order, "status": "waiting_for_entry_or_position_read"})
            continue
        entries = [d for d in ds["records"] if d["entry"] in (0, 2)
                   and _positive_id(d["position_id"]) and d["volume"] > 0]
        if not entries:
            history_has_position = (hs["status"] == "complete"
                and any(_positive_id(h["position_id"]) for h in hs["records"]))
            out.append({"order_ticket": order,
                "status": "waiting_for_entry_deals" if history_has_position else "no_entry_deals_observed"})
            continue
        for d in entries:
            matches = [p for p in positions["records"] if p["identifier"] == d["position_id"]]
            valid = (len(matches) == 1 and matches[0]["symbol"] == d["symbol"]
                     and matches[0]["magic"] == d["magic"])
            out.append({"order_ticket": order, "entry_deal_ticket": d["ticket"],
                "position_identifier": d["position_id"],
                "status": "observed_current_position" if valid else "waiting_or_conflicting_current_position",
                "physical_position_tickets": [p["ticket"] for p in matches]})
    return out


def collect_pending_evidence(api, *, expected_account, operation_token,
                             local_observation_seq, bindings):
    """Collect raw evidence through an injected, previously bound READ API.

    No clock argument can assert a broker history watermark. The sequence/token
    are caller correlation data only. The future Bridge must bind them to its
    still-current ExecutionSession, validate volume units and publish outside
    Widok. This function cannot construct PendingCancelObservationV1 as ready.
    """
    expected = _identity(expected_account)
    if (expected is None or not isinstance(operation_token, str) or not operation_token
            or not _positive_id(local_observation_seq)):
        raise ValueError("valid expected account, opaque token and local sequence required")
    scoped = _bindings(bindings)
    before = _account(api)
    result = {"schema": SCHEMA, "mode": "evidence_only", "operation_token_echo": operation_token,
        "local_observation_seq": local_observation_seq, "app_generation_verified": False,
        "expected_account": expected, "account_before": before, "account_after": None,
        "identity_consistent": False, "volume_model": "unverified",
        "broker_history_read_through_msc": None,
        "history_cutoff_status": "not_proven_by_read_api",
        "max_observed_broker_timestamp_msc": None,
        "current_orders": {"status": "not_attempted", "records": None},
        "orders": [], "positions": {"status": "not_attempted", "records": None},
        "entry_position_links": [], "proof_status": "not_evaluated_not_qualified",
        "status": "requires_review"}
    if before["status"] != "complete" or before["identity"] != expected:
        # Never collect another account's history to make the requested proof fit.
        return result
    result["current_orders"] = _read(api.orders_get, "order")
    for binding in scoped:
        ticket = binding["order_ticket"]
        result["orders"].append({"binding": binding,
            "history": _read(api.history_orders_get, "order", binding, ticket=ticket),
            "deals": _read(api.history_deals_get, "deal", binding, ticket=ticket)})
    result["positions"] = _read(api.positions_get, "position")
    after = _account(api)
    result["account_after"] = after
    result["identity_consistent"] = after["status"] == "complete" and after["identity"] == expected
    reads = [result["current_orders"], result["positions"]]
    reads += [r[k] for r in result["orders"] for k in ("history", "deals")]
    if result["identity_consistent"]:
        statuses = {r["status"] for r in reads}
        result["status"] = ("requires_review" if statuses & {"error", "invalid"}
                            else "waiting" if "none" in statuses else "raw_complete")
        if result["status"] != "requires_review":
            result["entry_position_links"] = _links(result["orders"], result["positions"])
    times = [row[k] for read in reads if read["status"] == "complete" for row in read["records"]
             for k in ("time_setup_msc", "time_done_msc", "time_msc")
             if _positive_id(row.get(k))]
    if times:
        result["max_observed_broker_timestamp_msc"] = max(times)
    # Successful lists and the greatest observed timestamp are NOT evidence that
    # no later deal exists; leave the history cutoff/model/generation unqualified.
    return result
