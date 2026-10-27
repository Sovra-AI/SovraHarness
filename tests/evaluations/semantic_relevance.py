"""Opt-in synthetic embedding calibration, separate from application memory.

Standard library only. Uses the active official OpenRouter connection's development
credential, with an explicitly named embedding model. Makes billable requests.
Never reads chat/memory databases or writes application preferences. Stores scores
and timing, not credentials, provider request IDs or embedding vectors.
"""

import argparse
import json
import math
from pathlib import Path
import time
import urllib.error
import urllib.request


FACTS = [
    ("mira_tea", "Mira prefers rooibos tea."),
    ("leo_tea", "Leo prefers green tea."),
    ("mira_role", "Mira is the user's colleague on project Cedar."),
    ("cedar_day", "Project Cedar sends its newsletter on Tuesday."),
    ("cedar_frequency", "Project Cedar's newsletter is sent monthly."),
    ("birch_day", "Project Birch sends its newsletter on Friday."),
    ("food", "The user avoids peanuts because of an allergy."),
    ("seat", "The user prefers an aisle seat when flying."),
    ("city", "The user lives in Porto."),
    ("review", "Project Juniper requires two reviewers for changes."),
    ("language", "The user prefers replies in Spanish."),
    ("time", "The user works in the Asia/Dubai time zone."),
    ("dark", "The user prefers a dark interface theme."),
    ("tests", "Project Willow runs cargo nextest before merging changes."),
    ("meeting", "The user keeps Wednesday mornings free of meetings."),
    ("budget", "Project Maple has a monthly hosting budget of forty dollars."),
] + [(f"inventory_{i}", f"Synthetic inventory item {i} is stored on shelf {i % 7}.") for i in range(100)]

# Labels are fixed before any requests. Empty labels require abstention. These are
# individual supporting facts, not a claim that a reader will answer correctly.
QUERIES = [
    ("literal_tea", "Which tea does Mira prefer?", ["mira_tea"]),
    ("paraphrase_tea", "What hot drink should I serve my colleague Mira?", ["mira_tea", "mira_role"]),
    ("wrong_person", "What tea does Nadia like?", []),
    ("literal_schedule", "Which day does Cedar send its newsletter?", ["cedar_day"]),
    ("paraphrase_frequency", "How often does the Cedar bulletin go out?", ["cedar_frequency"]),
    ("multi_fact", "When and how frequently is Cedar's newsletter delivered?", ["cedar_day", "cedar_frequency"]),
    ("wrong_project", "When is Aspen's newsletter sent?", []),
    ("negative_attribute", "What color is Mira's bicycle?", []),
    ("food_paraphrase", "Which ingredient must you leave out when cooking for me?", ["food"]),
    ("seat_paraphrase", "Should you book me a window or aisle on the plane?", ["seat"]),
    ("city_paraphrase", "Which city is my home?", ["city"]),
    ("review_paraphrase", "How many people must approve a Juniper change?", ["review"]),
    ("language_paraphrase", "What language should you use when answering me?", ["language"]),
    ("time_paraphrase", "What time zone should my calendar use?", ["time"]),
    ("dark_paraphrase", "Would I rather have light or dark app colors?", ["dark"]),
    ("tests_paraphrase", "What check must pass before a Willow merge?", ["tests"]),
    ("meeting_paraphrase", "When should you avoid scheduling a morning call for me?", ["meeting"]),
    ("budget_paraphrase", "What's Maple's monthly limit for server costs?", ["budget"]),
    ("spanish", "¿Qué infusión prefiere Mira?", ["mira_tea"]),
    ("japanese", "私が住んでいる都市はどこですか？", ["city"]),
    ("arabic", "ما المنطقة الزمنية التي أعمل بها؟", ["time"]),
    ("unrelated", "How tall is Mount Everest?", []),
    ("unsupported_history", "What city did I live in before Porto?", []),
    ("unsupported_reason", "Why does Mira prefer rooibos?", []),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dev-data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--model", required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    saved = json.loads((args.dev_data / "models/connections.json").read_text())
    active = saved["preferences"]["active"]
    connection = next(c for c in saved["connections"] if c["id"] == active["connection_id"])
    if connection["kind"] != "OpenRouter" or connection["base_url"].rstrip("/") != "https://openrouter.ai/api/v1":
        raise SystemExit("Requires the active official OpenRouter connection.")
    identifier = connection["id"]
    if not identifier or any(not (c.isascii() and (c.isalnum() or c == "-")) for c in identifier):
        raise SystemExit("Invalid connection identifier.")
    secret = (args.dev_data / "dev-credentials" / identifier).read_text().strip()
    report = {"suite": "synthetic-semantic-relevance-v1", "model": args.model,
              "status": "running", "facts": dict(FACTS), "requests": [], "queries": [],
              "notes": "Direct embeddings, normalized exact cosine ranking over synthetic facts. No HNSW, lexical fusion, temporal/source validation, extraction or reader. Query requests are sequential; latency includes HTTP connection setup. Scores do not establish entailment."}
    path = args.output / "report.json"

    def save():
        path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")

    dimension = None

    def embed(texts, purpose):
        nonlocal dimension
        request = urllib.request.Request("https://openrouter.ai/api/v1/embeddings",
            data=json.dumps({"model": args.model, "input": texts, "encoding_format": "float"}).encode(),
            headers={"Authorization": "Bearer " + secret, "Content-Type": "application/json"})
        observation = {"purpose": purpose, "input_count": len(texts)}
        started = time.monotonic()
        result = None
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                data = response.read(2 * 1024 * 1024 + 1)
                if len(data) > 2 * 1024 * 1024:
                    raise ValueError("Response limit")
                result = json.loads(data)
                usage = result.get("usage", {})
                observation.update({"http_status": response.status, "prompt_tokens": usage.get("prompt_tokens"), "cost": usage.get("cost")})
        except urllib.error.HTTPError as error:
            observation["http_status"] = error.code
        except (urllib.error.URLError, TimeoutError, ValueError):
            observation["request_failed"] = True
        observation["elapsed_ms"] = round((time.monotonic() - started) * 1000)
        report["requests"].append(observation)
        save()
        try:
            rows = result["data"]
            assert len(rows) == len(texts)
            assert sorted(row["index"] for row in rows) == list(range(len(texts)))
            vectors = []
            for row in sorted(rows, key=lambda row: row["index"]):
                vector = row["embedding"]
                assert 0 < len(vector) <= 4096 and all(math.isfinite(x) for x in vector)
                norm = math.sqrt(sum(x*x for x in vector))
                assert norm > 0
                dimension = dimension or len(vector)
                assert len(vector) == dimension
                vectors.append([x/norm for x in vector])
            report["dimension"] = dimension
            return vectors
        except (KeyError, TypeError, ValueError, AssertionError):
            report["status"] = "request_failed"
            save()
            raise SystemExit("Invalid or failed response; redacted partial report retained.") from None

    save()
    vectors = []
    for start in range(0, len(FACTS), 8):
        vectors.extend(embed([text for _, text in FACTS[start:start+8]], "facts"))
    for name, question, relevant in QUERIES:
        query = embed([question], name)[0]
        ranked = sorted([{"fact": identifier, "cosine": sum(x*y for x, y in zip(query, vector))}
                         for (identifier, _), vector in zip(FACTS, vectors)], key=lambda row: row["cosine"], reverse=True)
        report["queries"].append({"case": name, "question": question, "relevant": relevant,
                                  "ranking": ranked[:24], "relevant_scores": {r["fact"]: r["cosine"] for r in ranked if r["fact"] in relevant}})
        save()
    report["thresholds"] = []
    for threshold in [0.3, 0.4, 0.5, 0.6, 0.7, 0.8]:
        selected = true = positives = abstained = 0
        for case in report["queries"]:
            admitted = [row["fact"] for row in case["ranking"] if row["cosine"] >= threshold]
            true += len(set(admitted) & set(case["relevant"]))
            selected += len(admitted)
            positives += len(case["relevant"])
            abstained += int(not case["relevant"] and not admitted)
        report["thresholds"].append({"floor": threshold, "relevant_retrieved": true,
            "relevant_total": positives, "admitted": selected, "negative_queries_abstained": abstained,
            "negative_queries_total": sum(not q["relevant"] for q in report["queries"])})
    report["status"] = "observations_complete"
    save()
    print(path)


if __name__ == "__main__":
    main()
