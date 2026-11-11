"""Keep benchmark selection and evidence ownership independent of retrieval."""

import unittest
from pathlib import Path
import tempfile
from unittest.mock import patch

from prepare_longmemeval import prepare, timestamp, write_fixture


def question(identifier, category="single-session-user"):
    return {
        "question_id": identifier, "question_type": category,
        "question": "Where did I move?", "answer": "Lisbon",
        "question_date": "2023/05/31 (Wed) 10:00",
        "haystack_dates": ["2023/05/30 (Tue) 10:00"],
        "haystack_session_ids": ["session"], "answer_session_ids": ["session"],
        "haystack_sessions": [[
            {"role": "user", "content": "I moved to Lisbon.", "has_answer": True},
            {"role": "assistant", "content": "I suggested Porto.", "has_answer": True},
            {"role": "user", "content": "Unrelated distraction."},
        ]],
    }


class PreparationTests(unittest.TestCase):
    def test_sorted_strata_include_abstention_and_full_distracting_history(self):
        data = [question("z"), question("a"), question("b_abs")]
        result = prepare(data, 1)
        self.assertEqual([case["id"] for case in result["cases"]], ["b_abs", "a"])
        self.assertEqual(len(result["cases"][1]["sources"]), 2)

    def test_labels_and_assistant_text_never_become_indexed_user_evidence(self):
        case = prepare([question("a")], 1)["cases"][0]
        self.assertEqual(case["expected_source_keys"], ["0:session:0"])
        self.assertEqual(case["excluded_assistant_evidence"], 1)
        self.assertEqual(case["sources"][1]["text"], "Unrelated distraction.")
        self.assertTrue(all(set(source) == {"key", "session", "text", "time"}
                            for source in case["sources"]))
        self.assertNotIn("answer", case)

    def test_mismatched_session_metadata_is_rejected(self):
        row = question("a")
        row["haystack_dates"] = []
        with self.assertRaises(ValueError):
            prepare([row], 1)

    def test_repeated_upstream_session_ids_remain_distinct_occurrences(self):
        row = question("a")
        row["haystack_session_ids"].append("session")
        row["haystack_dates"].append("2023/05/29 (Mon) 10:00")
        row["haystack_sessions"].append([
            {"role": "user", "content": "Older distraction."}
        ])
        case = prepare([row], 1)["cases"][0]
        self.assertEqual(len({source["key"] for source in case["sources"]}), 3)
        self.assertEqual(len({source["session"] for source in case["sources"]}), 2)

    def test_dates_have_a_declared_timezone(self):
        self.assertEqual(timestamp("1970/01/01 (Thu) 00:00"), 0)

    def test_failed_write_never_publishes_or_replaces_a_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.json"
            with patch("prepare_longmemeval.json.dump", side_effect=OSError("disk full")):
                with self.assertRaises(OSError):
                    write_fixture(path, {})
            self.assertEqual(list(path.parent.iterdir()), [])
            write_fixture(path, {"complete": True})
            with self.assertRaises(FileExistsError):
                write_fixture(path, {})
            self.assertEqual(path.read_text(), '{"complete": true}')


if __name__ == "__main__":
    unittest.main()
