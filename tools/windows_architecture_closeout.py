#!/usr/bin/env python3
"""Reconcile a pinned Windows architecture triage queue with exact-blob reviews.

The original 307-path queue is historical and remains byte-for-byte unchanged.
Only explicitly reviewed source blobs receive a disposition in the 322-path
integration snapshot. A PR or a passing suite alone never marks a file reviewed.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import re
from collections import Counter
from pathlib import Path

from windows_architecture_ledger import FIELDS, render

PIN = "b9b3c10ac9054e4c4baba1b28c6026c65b38944c"
OLD_SHA256 = "e72afe22b9abbf4a8745a1add0d3a92eb1464a05c56a24994b813d383285b978"
AUDIT = Path("docs/audits")
EVIDENCE = AUDIT / "windows-2026-10-18-reviewed-blobs.csv"
OUTPUT = AUDIT / "windows-2026-10-18-b9b3c10a-ledger.csv"
SUMMARY = AUDIT / "windows-2026-10-18-b9b3c10a-ledger.summary.json"
EXTRAS = ("review_evidence", "review_disposition", "review_owner", "independent_falsifier")
STATUS = {"REVIEWED_NO_CHANGE", "REVIEWED_CONDITIONAL_RISK"}
EVIDENCE_FIELDS = ("path", "git_blob_sha", *EXTRAS)
EVIDENCE_URL = re.compile(
    r"^https://github\.com/gyldlab/keld/(?:issues|pull)/[0-9]+"
    r"(?:#issuecomment-[0-9]+)?$"
)


def reconcile(source_csv: str, evidence_csv: str) -> tuple[str, dict[str, int]]:
    """Reject stale, duplicate, malformed or ungrounded review attestations."""
    original = csv.DictReader(io.StringIO(source_csv))
    if original.fieldnames != list(FIELDS):
        raise ValueError("unexpected base ledger fields")
    rows = list(original)
    if len({row["path"] for row in rows}) != len(rows):
        raise ValueError("duplicate candidate path in source ledger")
    known = {row["path"]: row for row in rows}

    claims = csv.DictReader(io.StringIO(evidence_csv))
    if claims.fieldnames != list(EVIDENCE_FIELDS):
        raise ValueError("unexpected review evidence fields")
    reviewed: set[str] = set()
    for claim in claims:
        path = claim["path"]
        if path in reviewed:
            raise ValueError(f"duplicate review for {path}")
        reviewed.add(path)
        if path not in known:
            raise ValueError(f"review targets absent candidate {path}")
        if claim["git_blob_sha"] != known[path]["git_blob_sha"]:
            raise ValueError(f"stale review blob for {path}")
        disposition = claim["review_disposition"]
        if disposition not in STATUS:
            raise ValueError(f"unrecognized disposition for {path}")
        if not EVIDENCE_URL.fullmatch(claim["review_evidence"]):
            raise ValueError(f"missing public evidence for {path}")
        if not claim["review_owner"].strip() or not claim["independent_falsifier"].strip():
            raise ValueError(f"missing owner or independent falsifier for {path}")
        for field in EXTRAS:
            known[path][field] = claim[field]
        known[path]["review_status"] = disposition

    writer_output = io.StringIO(newline="")
    writer = csv.DictWriter(writer_output, fieldnames=(*FIELDS, *EXTRAS), lineterminator="\n")
    writer.writeheader()
    for row in rows:
        for field in EXTRAS:
            row.setdefault(field, "")
        writer.writerow(row)
    statuses = Counter(row["review_status"] for row in rows)
    permitted = STATUS | {"PENDING_INDEPENDENT_SEMANTIC_REVIEW"}
    if set(statuses) - permitted:
        raise ValueError("source ledger contains unknown review statuses")
    summary = {
        "candidates": len(rows),
        "reviewed_no_change": statuses["REVIEWED_NO_CHANGE"],
        "reviewed_conditional_risk": statuses["REVIEWED_CONDITIONAL_RISK"],
        "reviewed_total": sum(statuses[status] for status in STATUS),
        "pending_independent_review": statuses["PENDING_INDEPENDENT_SEMANTIC_REVIEW"],
        "not_closed": len(rows) - statuses["REVIEWED_NO_CHANGE"],
    }
    if summary["reviewed_total"] != len(reviewed) or sum(statuses.values()) != len(rows):
        raise ValueError("incomplete source/evidence reconciliation")
    return writer_output.getvalue(), summary


def snapshot(root: Path) -> tuple[bytes, bytes]:
    previous = root / AUDIT / "windows-2026-10-17-triage.csv"
    original_hash = hashlib.sha256(previous.read_bytes().replace(b"\r\n", b"\n")).hexdigest()
    if original_hash != OLD_SHA256:
        raise ValueError("historical #891 baseline changed; do not silently overwrite")
    source, base_summary = render(root, PIN)
    claims_path = root / EVIDENCE
    claims = claims_path.read_bytes().replace(b"\r\n", b"\n")
    ledger, counts = reconcile(source, claims.decode("utf-8"))
    digest = hashlib.sha256(ledger.encode("utf-8")).hexdigest()
    summary = {
        "schema": "keld.windows-architecture-closeout/v1",
        "pinned_ref": PIN,
        "review_scope": "Windows-related tracked candidate paths only, not whole #824",
        "review_provenance": "exact per-file Git blob and public independent-review citation",
        "historical_307_queue_sha256": OLD_SHA256,
        "evidence_sha256": hashlib.sha256(claims).hexdigest(),
        "manifest_sha256": digest,
        "base_class_counts": base_summary["classes"],
        **counts,
    }
    if counts["candidates"] != base_summary["tracked_candidates"]:
        raise ValueError("source generator count drift")
    return ledger.encode("utf-8"), (json.dumps(summary, sort_keys=True, indent=2) + "\n").encode("utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--check", action="store_true", help="refuse any stale output without rewriting")
    args = parser.parse_args()
    root = args.repo.resolve()
    ledger, summary = snapshot(root)
    for path, expected in ((OUTPUT, ledger), (SUMMARY, summary)):
        dest = root / path
        if args.check:
            existing = dest.read_bytes().replace(b"\r\n", b"\n")
            if existing != expected:
                raise ValueError(f"stale snapshot output: {path}")
        else:
            dest.write_bytes(expected)
    print(summary.decode("utf-8").strip())


if __name__ == "__main__":
    main()
