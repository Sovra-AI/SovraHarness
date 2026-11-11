import unittest

from summarize_retrieval import summarize


class SummaryTests(unittest.TestCase):
    def test_incomplete_runs_never_receive_a_score(self):
        for status in ("running", "failed", None):
            with self.assertRaises(ValueError):
                summarize({"status": status})

    def test_unsupported_and_abstention_cases_are_not_successes_or_failures(self):
        metric_names = ("recall_at_1", "recall_at_4", "recall_at_12", "context_source_recall")
        report = {"suite": "longmemeval-s-user-source-v1", "status": "complete",
                  "scope": "source retrieval", "fixture_sha256": "fixture", "cases": [
                      {"category": "multi-session", "expected_sources": 2,
                       "scores": {name: 0.5 for name in metric_names}},
                      {"category": "single-session-assistant", "expected_sources": 0, "scores": None},
                      {"category": "abstention", "expected_sources": 1, "scores": None},
                  ]}
        overall = summarize(report)["categories"]["overall"]
        self.assertEqual(overall["questions"], 3)
        self.assertEqual(overall["scored_questions"], 1)
        self.assertEqual(overall["labeled_user_sources_in_scored_questions"], 2)
        self.assertEqual(overall["metrics"]["recall_at_4"]["macro_mean"], 0.5)
        report["cases"][0]["scores"]["recall_at_4"] = None
        with self.assertRaises(ValueError):
            summarize(report)


if __name__ == "__main__":
    unittest.main()
