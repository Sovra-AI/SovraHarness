#!/usr/bin/env python3
"""Summarize only completed user-source retrieval diagnostics, never QA accuracy."""

import argparse
from collections import defaultdict
import json
from pathlib import Path


def summarize(report):
    if report.get("status") != "complete":
        raise ValueError("An incomplete diagnostic cannot receive an aggregate score")
    if report.get("suite") != "longmemeval-s-user-source-v1":
        raise ValueError("Unsupported diagnostic suite")
    groups = defaultdict(list)
    for case in report["cases"]:
        groups[case["category"]].append(case)
    groups["overall"] = report["cases"]
    result = {}
    for category, cases in sorted(groups.items()):
        eligible = [c for c in cases if c["category"] != "abstention" and c["expected_sources"] > 0]
        metrics = {}
        for metric in ("recall_at_1", "recall_at_4", "recall_at_12", "context_source_recall"):
            values = [c["scores"][metric] for c in eligible]
            if any(value is None or not 0 <= value <= 1 for value in values):
                raise ValueError("Missing or invalid score in a case with labeled user evidence")
            metrics[metric] = {
                "macro_mean": sum(values) / len(values) if values else None,
                "all_labeled_sources_found_cases": sum(value == 1 for value in values),
            }
        result[category] = {
            "questions": len(cases), "scored_questions": len(eligible),
            "abstention_questions": sum(c["category"] == "abstention" for c in cases),
            "no_user_evidence_questions": sum(c["category"] != "abstention" and not c["expected_sources"] for c in cases),
            "labeled_user_sources_in_scored_questions": sum(c["expected_sources"] for c in eligible),
            "metrics": metrics,
        }
    return {"scope": report["scope"], "fixture_sha256": report["fixture_sha256"], "categories": result}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(json.loads(args.report.read_text())), indent=2))
