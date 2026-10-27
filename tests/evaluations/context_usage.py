"""Opt-in synthetic usage measurement; not used or packaged by the Rust application.

Uses only Python's standard library. Reads the active OpenRouter development
connection and its credential once; never opens chat/memory data or changes settings.
Requests are billable. A fresh output directory retains partial results on failure.
"""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct
import time
import urllib.error
import urllib.request
import zlib


def synthetic_png(width, height):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(
            ">I", zlib.crc32(kind + data) & 0xFFFFFFFF
        )

    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend((40 if x < width // 2 else 200, 80 if y < height // 2 else 160, 120))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dev-data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    saved = json.loads((args.dev_data / "models/connections.json").read_text())
    active = saved["preferences"]["active"]
    connection = next(item for item in saved["connections"] if item["id"] == active["connection_id"])
    if connection["kind"] != "OpenRouter" or connection["base_url"].rstrip("/") != "https://openrouter.ai/api/v1":
        raise SystemExit("This diagnostic requires the active official OpenRouter connection.")
    connection_id = connection["id"]
    if not connection_id or any(not (c.isascii() and (c.isalnum() or c == "-")) for c in connection_id):
        raise SystemExit("Invalid connection identifier.")
    secret = (args.dev_data / "dev-credentials" / connection_id).read_text().strip()
    model = active["model_id"]
    suffix = "\nReply with OK only."
    cases = [
        ("text_control", "", []),
        ("english", "Synthetic project Cedar sends a monthly newsletter. Mira likes rooibos tea.\n" * 200, []),
        ("multilingual", "هذه بيانات اصطناعية. 京都に住んでいます。 Café, naïve, résumé.\n" * 200, []),
        ("code_json", 'fn square(x: u64) -> u64 { x * x }\n{"project":"Cedar","active":true,"count":42}\n' * 200, []),
        ("image_200", "", [(200, 200)]),
        ("image_1000", "", [(1000, 1000)]),
        ("image_2048", "", [(2048, 2048)]),
        ("image_wide", "", [(2048, 512)]),
        ("eight_images", "", [(2048, 2048)] * 8),
    ]
    report = {"suite": "synthetic-context-usage-v1", "model": model, "connection_id": connection_id,
              "status": "running", "notes": "Direct non-streaming provider requests, no tools, no memory, no history. Native prompt usage is provider-reported. Not a full application admission measurement.",
              "observations": []}
    path = args.output / "report.json"

    def save():
        path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")

    save()
    for name, text, dimensions in cases:
        prompt = text + suffix
        images = [synthetic_png(width, height) for width, height in dimensions]
        content = [{"type": "text", "text": prompt}] + [
            {"type": "image_url", "image_url": {"url": "data:image/png;base64," + base64.b64encode(data).decode("ascii")}}
            for data in images
        ]
        body = {"model": model, "messages": [{"role": "user", "content": content}], "max_tokens": 32, "stream": False}
        observation = {"case": name, "text": prompt, "text_bytes": len(prompt.encode()),
                       "images": [{"width": w, "height": h, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                                  for (w, h), data in zip(dimensions, images)],
                       "text_and_image_reservation_units": len(prompt.encode()) + len(images) * 8192}
        request = urllib.request.Request("https://openrouter.ai/api/v1/chat/completions",
                                         data=json.dumps(body).encode(),
                                         headers={"Authorization": "Bearer " + secret, "Content-Type": "application/json"})
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=90) as response:
                result = json.load(response)
                usage = result.get("usage", {})
                observation.update({"http_status": response.status,
                                    "prompt_tokens": usage.get("prompt_tokens"),
                                    "completion_tokens": usage.get("completion_tokens"),
                                    "cost": usage.get("cost"),
                                    "finish_reason": result.get("choices", [{}])[0].get("finish_reason")})
        except urllib.error.HTTPError as error:
            observation["http_status"] = error.code
        except (urllib.error.URLError, TimeoutError):
            observation["transport_failed"] = True
        observation["elapsed_ms"] = round((time.monotonic() - started) * 1000)
        report["observations"].append(observation)
        failed = observation.get("http_status") != 200 or observation.get("prompt_tokens") is None
        report["status"] = "request_failed" if failed else "running"
        save()
        if failed:
            raise SystemExit("Request failed; redacted partial report retained.")
    report["status"] = "observations_complete"
    save()
    print(path)


if __name__ == "__main__":
    main()
