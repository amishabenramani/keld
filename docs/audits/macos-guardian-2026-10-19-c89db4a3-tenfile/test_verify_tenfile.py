"""Mutation controls for the Guardian ten-file exact-blob audit delta."""

from __future__ import annotations

import csv
import json
import shutil
import tempfile
import unittest
from pathlib import Path

from verify_tenfile import HERE, ROOT, Invalid, verify


class SourceAuditVerifierTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory(prefix="guardian-audit-ten-")
        self.folder = Path(self.scratch.name)
        for name in ("ledger_77.csv", "review_append.csv", "summary.json"):
            shutil.copyfile(HERE / name, self.folder / name)

    def tearDown(self) -> None:
        self.scratch.cleanup()

    def mutate_csv(self, name: str, change) -> None:
        path = self.folder / name
        with path.open(newline="", encoding="utf-8") as stream:
            reader = csv.DictReader(stream)
            columns, values = reader.fieldnames, list(reader)
        change(values)
        with path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=columns)
            writer.writeheader()
            writer.writerows(values)

    def mutate_summary(self, change) -> None:
        path = self.folder / "summary.json"
        contents = json.loads(path.read_text(encoding="utf-8"))
        change(contents)
        path.write_text(json.dumps(contents))

    def assert_invalid(self) -> None:
        with self.assertRaises(Invalid):
            verify(ROOT, self.folder)

    def test_valid_immutable_source_evidence(self) -> None:
        verify(ROOT, self.folder)

    def test_duplicate_review_path_rejected(self) -> None:
        self.mutate_csv("review_append.csv", lambda values: values.append(dict(values[0])))
        self.assert_invalid()

    def test_forged_historical_blob_rejected(self) -> None:
        self.mutate_csv("review_append.csv", lambda values: values[0].__setitem__("blob_949", "0" * 40))
        self.assert_invalid()

    def test_forged_current_blob_rejected(self) -> None:
        self.mutate_csv("review_append.csv", lambda values: values[1].__setitem__("blob_956", "f" * 40))
        self.assert_invalid()

    def test_invented_source_path_rejected(self) -> None:
        self.mutate_csv("review_append.csv", lambda values: values[0].__setitem__("path", "crates/keld-runtime/src/macos_guardian/imaginary.rs"))
        self.assert_invalid()

    def test_original_parent_ledger_field_immutable(self) -> None:
        self.mutate_csv("ledger_77.csv", lambda values: values[0].__setitem__("blob_952", "0" * 40))
        self.assert_invalid()

    def test_unreviewed_historical_row_cannot_be_promoted(self) -> None:
        def promote(values):
            row = next(r for r in values if r["followon_source_status"] == "pending-source-review" and not r["batch10_evidence_file"])
            row["batch10_source_status"] = "source-reviewed-limited-batch10"
        self.mutate_csv("ledger_77.csv", promote)
        self.assert_invalid()

    def test_changed_overlay_cannot_be_certified_historical(self) -> None:
        def promote(values):
            row = next(r for r in values if r["path"].endswith("authenticated_runner.rs"))
            row["batch10_source_status"] = "source-reviewed-limited-batch10"
        self.mutate_csv("ledger_77.csv", promote)
        self.assert_invalid()

    def test_unmatched_disposition_cannot_claim_evidence(self) -> None:
        def rewrite(values):
            row = next(r for r in values if r["batch10_evidence_file"])
            row["batch10_disposition"] = "unreviewed replacement claim"
        self.mutate_csv("ledger_77.csv", rewrite)
        self.assert_invalid()

    def test_empty_invariant_cannot_claim_source_review(self) -> None:
        self.mutate_csv("review_append.csv", lambda values: values[0].__setitem__("invariant", ""))
        self.assert_invalid()

    def test_false_native_test_rerun_rejected(self) -> None:
        self.mutate_summary(lambda doc: doc.__setitem__("native_tests_rerun_for_documentation", True))
        self.assert_invalid()

    def test_false_all_files_complete_rejected(self) -> None:
        self.mutate_summary(lambda doc: doc.__setitem__("all_77_source_complete", True))
        self.assert_invalid()

    def test_false_merge_authorization_rejected(self) -> None:
        self.mutate_summary(lambda doc: doc.__setitem__("merge_authorized", True))
        self.assert_invalid()

    def test_changed_source_overlay_remains_pending(self) -> None:
        self.mutate_summary(lambda doc: doc.__setitem__("changed_historical_full_review_still_pending", []))
        self.assert_invalid()

    def test_false_pending_count_rejected(self) -> None:
        self.mutate_summary(lambda doc: doc.__setitem__("pending_historical_full_review", 0))
        self.assert_invalid()


if __name__ == "__main__":
    unittest.main()
