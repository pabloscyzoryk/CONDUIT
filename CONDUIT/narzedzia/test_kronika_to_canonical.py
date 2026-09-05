"""Synthetic ingress regressions; no account or channel history is embedded."""
import json
from pathlib import Path
import tempfile
import unittest

from kronika_to_canonical import build_payload_and_manifest


class ChronicleContract(unittest.TestCase):
    def convert(self, records, marker=False):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "kronika.txt"
            prefix = "[KRONIKA] [PROV kronika.jsonl:1] " if marker else ""
            source.write_text("\n".join(prefix + json.dumps(row) for row in records), encoding="utf-8")
            return build_payload_and_manifest(
                [source], chat_id=-1001234567890, channel="Synergy",
                selected_day=None, utc_offset_minutes=0, telegram_export=None,
            )

    @staticmethod
    def row(kind, msg=1, received=1000000000000, text="BUY LIMITS GOLD @ 2100/2095"):
        row = dict(rodzaj=kind, msg_id=msg, chat_id=-1001234567890,
                   odebrano_ms=received, ts_telegram_ms=received - 1000,
                   text=text, seq=1)
        if kind == "edycja":
            row["edit_of"] = msg
        return row

    def test_empty_edit_cancels_payload_instead_of_disappearing(self):
        rows = [self.row("nowa"), self.row("edycja", received=1000000001000, text="")]
        payload, manifest = self.convert(rows)
        self.assertEqual([row["event"] for row in payload["messages"]], ["new", "edit"])
        self.assertEqual(payload["messages"][1]["text"], "")
        self.assertEqual(payload["messages"][1]["edit_of"], 1)
        self.assertEqual(manifest["counts"]["messages_emitted"], 2)

    def test_receive_clock_orders_events_and_keeps_duplicate_edits(self):
        first = self.row("nowa", msg=20)
        second = self.row("edycja", msg=20, received=1000000001000)
        second["ts_telegram_ms"] = first["ts_telegram_ms"] - 10000
        payload, _ = self.convert([first, second, second], marker=True)
        messages = payload["messages"]
        self.assertEqual(len(messages), 3)
        self.assertEqual([m["receive_seq"] for m in messages], [1, 2, 3])
        self.assertEqual([m["ts"] for m in messages], [1000000000000, 1000000001000, 1000000001000])

    def test_direct_reply_and_orphan_edit_identity_are_preserved(self):
        row = self.row("edycja", msg=12)
        row["reply_to"] = 8
        payload, _ = self.convert([row])
        message = payload["messages"][0]
        self.assertEqual((message["reply_to"], message["edit_of"]), (8, 12))

    def test_empty_new_is_skipped_and_delete_is_not(self):
        rows = [self.row("nowa", text=""), self.row("skasowana", text="")]
        payload, _ = self.convert(rows)
        self.assertEqual([m["event"] for m in payload["messages"]], ["delete"])


if __name__ == "__main__":
    unittest.main()
