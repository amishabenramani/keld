"""Mutation-negative controls for immutable eight-file Guardian evidence."""

import csv
import json
import shutil
import tempfile
import unittest
from pathlib import Path

from verify_eightfile import HERE, ROOT, Invalid, verify


class EightFileEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.owned = tempfile.TemporaryDirectory(prefix="keld-guardian-eight-audit-")
        self.folder = Path(self.owned.name)
        for name in ("ledger_77.csv", "review_append.csv", "summary.json"):
            shutil.copyfile(HERE / name, self.folder / name)

    def tearDown(self) -> None:
        self.owned.cleanup()

    def edit_csv(self, name: str, change) -> None:
        path = self.folder / name
        with path.open(newline="", encoding="utf-8") as stream:
            original = csv.DictReader(stream)
            columns, values = original.fieldnames, list(original)
        change(values)
        with path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=columns, lineterminator="\n")
            writer.writeheader()
            writer.writerows(values)

    def edit_summary(self, change) -> None:
        path = self.folder / "summary.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        change(value)
        path.write_text(json.dumps(value))

    def must_fail(self) -> None:
        with self.assertRaises(Invalid):
            verify(ROOT, self.folder)

    def test_valid_immutable_source_receipts(self) -> None:
        verify(ROOT, self.folder)

    def test_duplicate_receipt_path_denied(self) -> None:
        self.edit_csv("review_append.csv", lambda rows: rows.append(dict(rows[-1])))
        self.must_fail()

    def test_forged_original_blobs_denied(self) -> None:
        self.edit_csv("review_append.csv", lambda rows: rows[0].__setitem__("blob_949", "0" * 40))
        self.must_fail()

    def test_forged_source_tip_blobs_denied(self) -> None:
        self.edit_csv("review_append.csv", lambda rows: rows[0].__setitem__("blob_952", "f" * 40))
        self.must_fail()

    def test_unreviewed_historical_path_cannot_gain_status(self) -> None:
        def promote(rows):
            row = next(r for r in rows if r["batch8_source_status"] == "pending-source-review")
            row["batch8_source_status"] = "source-reviewed-limited-batch8"
        self.edit_csv("ledger_77.csv", promote)
        self.must_fail()

    def test_changed_old_source_cannot_be_closed(self) -> None:
        def promote(rows):
            row = next(r for r in rows if r["path"].endswith("/authenticated_runner.rs"))
            row["batch8_source_status"] = "source-reviewed-limited-batch8"
        self.edit_csv("ledger_77.csv", promote)
        self.must_fail()

    def test_inherited_source_evidence_must_not_change(self) -> None:
        self.edit_csv("ledger_77.csv", lambda rows: rows[0].__setitem__("batch10_disposition", "rewritten prior proof"))
        self.must_fail()

    def test_receipt_join_requires_same_disposition(self) -> None:
        def rewrite(rows):
            row = next(r for r in rows if r["batch8_evidence_file"])
            row["batch8_disposition"] = "invented verified physical custody"
        self.edit_csv("ledger_77.csv", rewrite)
        self.must_fail()

    def test_nonexistent_review_path_rejected(self) -> None:
        self.edit_csv("review_append.csv", lambda rows: rows[0].__setitem__("path", "crates/keld-runtime/src/macos_guardian/fake.rs"))
        self.must_fail()

    def test_missing_security_invariant_rejected(self) -> None:
        self.edit_csv("review_append.csv", lambda rows: rows[0].__setitem__("invariant", ""))
        self.must_fail()

    def test_late_r_original_ack_clock_anchor_required(self) -> None:
        def drop_r_cut(rows):
            row = next(r for r in rows if r["path"].endswith("/retained_ack.rs"))
            row["source_lines"] = row["source_lines"].replace("31-79,", "53-79,")
        self.edit_csv("review_append.csv", drop_r_cut)
        self.must_fail()

    def test_original_native_receipt_clock_anchor_required(self) -> None:
        def drop_lf_cut(rows):
            row = next(r for r in rows if r["path"].endswith("/retained_ack/observers.rs"))
            row["source_lines"] = row["source_lines"].replace("35-76,", "47-76,")
        self.edit_csv("review_append.csv", drop_lf_cut)
        self.must_fail()

    def test_actual_ack_release_and_native_group_retirement_cannot_be_omitted(self) -> None:
        def shrink(rows):
            row = next(r for r in rows if r["path"].endswith("/retained_ack.rs"))
            row["source_lines"] = row["source_lines"].replace("225-310", "225-225")
        self.edit_csv("review_append.csv", shrink)
        self.must_fail()

    def test_dual_death_original_birth_and_group_membership_cannot_be_omitted(self) -> None:
        def shrink(rows):
            row = next(r for r in rows if r["path"].endswith("/dual_death.rs"))
            row["source_lines"] = row["source_lines"].replace("53-86", "53-53")
        self.edit_csv("review_append.csv", shrink)
        self.must_fail()

    def test_detached_independent_session_and_original_birth_cannot_be_omitted(self) -> None:
        def shrink(rows):
            row = next(r for r in rows if r["path"].endswith("/escaped_descendant.rs"))
            row["source_lines"] = row["source_lines"].replace("105-135", "105-125")
        self.edit_csv("review_append.csv", shrink)
        self.must_fail()

    def test_successful_lf_cannot_hide_missing_read_and_accept_deadline_checks(self) -> None:
        def shrink(rows):
            row = next(r for r in rows if r["path"].endswith("/retained_ack/observers.rs"))
            row["source_lines"] = row["source_lines"].replace("35-76", "35-46")
        self.edit_csv("review_append.csv", shrink)
        self.must_fail()

    def test_denial_winner_no_locator_and_eof_assertions_cannot_be_omitted(self) -> None:
        def shrink(rows):
            row = next(r for r in rows if r["path"].endswith("/ownership_races.rs"))
            row["source_lines"] = row["source_lines"].replace("208-242", "208-231")
        self.edit_csv("review_append.csv", shrink)
        self.must_fail()

    def test_dual_death_pre_rescue_native_survivor_required(self) -> None:
        def remove_survivor(rows):
            row = next(r for r in rows if r["path"].endswith("/dual_death.rs"))
            row["source_lines"] = row["source_lines"].replace("87-108", "87-100")
        self.edit_csv("review_append.csv", remove_survivor)
        self.must_fail()

    def test_detached_child_before_test_only_rescue_cited(self) -> None:
        def remove_survival(rows):
            row = next(r for r in rows if r["path"].endswith("/escaped_descendant.rs"))
            row["source_lines"] = row["source_lines"].replace("136-177", "163-177")
        self.edit_csv("review_append.csv", remove_survival)
        self.must_fail()

    def test_unauthorized_stranger_native_sigkill_citation_required(self) -> None:
        def remove_sentinel(rows):
            row = next(r for r in rows if r["path"].endswith("/cleanup_tests.rs"))
            row["source_lines"] = row["source_lines"].replace("277-336,", "279-336,")
        self.edit_csv("review_append.csv", remove_sentinel)
        self.must_fail()

    def test_forged_historical_completed_count_rejected(self) -> None:
        self.edit_summary(lambda report: report.__setitem__("current_pending_historical", 0))
        self.must_fail()

    def test_forged_native_success_rejected(self) -> None:
        self.edit_summary(lambda report: report.__setitem__("native_suites_rerun", True))
        self.must_fail()

    def test_fake_kernel_custodian_certificate_rejected(self) -> None:
        self.edit_summary(lambda report: report.__setitem__("kernel_custody_proven", True))
        self.must_fail()

    def test_false_merge_permission_rejected(self) -> None:
        self.edit_summary(lambda report: report.__setitem__("merge_authorized", True))
        self.must_fail()

    def test_unreviewed_changed_source_count_must_remain(self) -> None:
        self.edit_summary(lambda report: report.__setitem__("changed_blobs_pending", []))
        self.must_fail()


if __name__ == "__main__":
    unittest.main()
