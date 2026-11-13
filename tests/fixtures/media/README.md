# Synthetic media

`ten-minute-blue.mp4` is a 600-second, 320×180 solid-blue video, one frame per
second, with no audio. Generated locally, not copied from a person or website:

```sh
ffmpeg -f lavfi -i color=c=blue:s=320x180:r=1 -t 600 -an \
  -c:v libx264 -threads 1 -movflags +faststart ten-minute-blue.mp4
```

It is intentionally highly compressed (19,083 bytes) so tests cannot confuse file
size with recording duration. Tests embed it only under test configuration; FFmpeg
is not needed to run the tests or application. SHA-256: `2e2a0a993181c1a6ae7894375b6f8c1e07c7d8e09ca19ae7e7db146505821877`.
