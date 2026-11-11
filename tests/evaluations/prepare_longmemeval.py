#!/usr/bin/env python3
"""Prepare a pinned, stratified LongMemEval-S source-retrieval diagnostic.

Uses the complete distracting history of each selected question. Answers and
evidence labels are scoring metadata, never indexed text. No model is called.
"""

import argparse
from collections import defaultdict
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import tempfile
import urllib.request

REVISION = "98d7416c24c778c2fee6e6f3006e7a073259d48f"
SHA256 = "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442"
SIZE = 277_383_467
URL = (
    "https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/"
    f"{REVISION}/longmemeval_s_cleaned.json"
)


def timestamp(value):
    # Upstream dates have no timezone. Use UTC consistently for this diagnostic;
    # do not infer valid-time filters from a natural-language question.
    return int(datetime.strptime(value, "%Y/%m/%d (%a) %H:%M")
               .replace(tzinfo=timezone.utc).timestamp() * 1000)


def prepare(data, per_category):
    groups = defaultdict(list)
    for row in data:
        category = "abstention" if row["question_id"].endswith("_abs") else row["question_type"]
        groups[category].append(row)
    cases = []
    for category, rows in sorted(groups.items()):
        for row in sorted(rows, key=lambda item: item["question_id"])[:per_category]:
            sources = []
            evidence = []
            excluded = 0
            for occurrence, (session_id, date, turns) in enumerate(zip(
                row["haystack_session_ids"], row["haystack_dates"], row["haystack_sessions"], strict=True
            )):
                # The released corpus repeats some distractor session IDs with
                # different dates. Preserve every occurrence without reusing a
                # producer identity for different evidence.
                session_key = f"{occurrence}:{session_id}"
                for index, turn in enumerate(turns):
                    assert turn["role"] in ("user", "assistant"), turn["role"]
                    if turn["role"] != "user":
                        excluded += int(turn.get("has_answer", False))
                        continue
                    key = f"{session_key}:{index}"
                    sources.append({"key": key, "session": session_key,
                                    "text": turn["content"], "time": timestamp(date)})
                    if turn.get("has_answer", False):
                        evidence.append(key)
            cases.append({"id": row["question_id"], "category": category,
                          "query": row["question"], "time": timestamp(row["question_date"]),
                          "sources": sources, "expected_source_keys": evidence,
                          "excluded_assistant_evidence": excluded,
                          "upstream_answer_sessions": row["answer_session_ids"]})
    return {"suite": "longmemeval-s-user-source-v1", "upstream": URL,
            "revision": REVISION, "upstream_sha256": SHA256,
            "selection": f"First {per_category} question IDs per category; abstention separate",
            "scope": "Full S histories, user-authored raw sources only; no extracted facts, embeddings, reader or judge. Not an official QA score.",
            "cases": cases}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dataset", type=Path, help="Optional already downloaded pinned upstream JSON")
    parser.add_argument("--per-category", type=int, default=4)
    args = parser.parse_args()
    if args.per_category < 1 or args.output.exists():
        parser.error("Use a positive category count and a fresh output file")
    if args.dataset:
        with args.dataset.open("rb") as stream:
            raw = stream.read(SIZE + 1)
    else:
        with urllib.request.urlopen(URL, timeout=120) as response:
            raw = response.read(SIZE + 1)
    if len(raw) != SIZE or hashlib.sha256(raw).hexdigest() != SHA256:
        raise ValueError("Upstream size/hash differs from the pinned dataset")
    fixture = prepare(json.loads(raw), args.per_category)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    write_fixture(args.output, fixture)
    print(json.dumps({"cases": len(fixture["cases"]), "output": str(args.output),
                      "fixture_sha256": hashlib.sha256(args.output.read_bytes()).hexdigest()}))


def write_fixture(output, fixture):
    # Publish only complete files, and never replace an existing benchmark input.
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=output.parent) as stream:
        json.dump(fixture, stream, ensure_ascii=False)
        stream.flush()
        os.fsync(stream.fileno())
        os.link(stream.name, output)


if __name__ == "__main__":
    main()
