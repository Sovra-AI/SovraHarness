# Synthetic media

`ten-minute-blue.mp4` is a 600-second, 320×180 solid-blue video, one frame per
second, with no audio. Generated locally, not copied from a person or website:

```sh
ffmpeg -f lavfi -i color=c=blue:s=320x180:r=1 -t 600 -an \
  -c:v libx264 -threads 1 -movflags +faststart ten-minute-blue.mp4
```

It is intentionally highly compressed (19,083 bytes) so tests cannot confuse file
size with recording duration. Tests embed it only under test configuration; FFmpeg
is not needed to run the tests or application. Hash and the original admission
investigation are in `docs/evaluations/memory-2026-10-27/media-duration.json`.
