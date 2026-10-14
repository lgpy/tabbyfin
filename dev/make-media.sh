#!/usr/bin/env bash
# Generates a small synthetic media library under dev/.data/media.
# Covers: a movie with two versions, a series with two seasons, multiple audio
# languages, embedded and external subtitles, and local artwork.
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)/.data/media"
dur="${DURATION:-60}"

srt() {
  local title="$1"
  printf '1\n00:00:01,000 --> 00:00:05,000\n%s\n\n2\n00:00:10,000 --> 00:00:15,000\nTen seconds in.\n\n3\n00:00:30,000 --> 00:00:35,000\nHalfway there.\n' "$title"
}

# video <out.mkv> <title> <size>
video() {
  local out="$1" title="$2" size="$3" sub
  [[ -f "$out" ]] && return
  mkdir -p "$(dirname "$out")"
  sub="$(mktemp --suffix=.srt)"
  srt "$title" > "$sub"
  ffmpeg -loglevel error -y \
    -f lavfi -i "testsrc2=size=${size}:rate=24:duration=${dur}" \
    -f lavfi -i "sine=frequency=440:duration=${dur}" \
    -f lavfi -i "sine=frequency=660:duration=${dur}" \
    -i "$sub" \
    -map 0:v -map 1:a -map 2:a -map 3:s \
    -vf "drawtext=text='${title}':fontcolor=white:fontsize=36:box=1:boxcolor=black@0.6:x=(w-tw)/2:y=h-80" \
    -c:v libx264 -preset ultrafast -crf 32 -pix_fmt yuv420p \
    -c:a aac -b:a 64k -c:s srt \
    -metadata title="$title" \
    -metadata:s:a:0 language=eng -metadata:s:a:0 title="English" \
    -metadata:s:a:1 language=jpn -metadata:s:a:1 title="Japanese" \
    -metadata:s:s:0 language=eng \
    "$out"
  rm -f "$sub"
}

# image <out.jpg> <label> <size> <color>
image() {
  local out="$1" label="$2" size="$3" color="$4"
  [[ -f "$out" ]] && return
  mkdir -p "$(dirname "$out")"
  ffmpeg -loglevel error -y -f lavfi -i "color=c=${color}:size=${size}" -frames:v 1 \
    -vf "drawtext=text='${label}':fontcolor=white:fontsize=48:x=(w-tw)/2:y=(h-th)/2" "$out"
}

movie="$root/Movies/Test Movie (2024)"
video "$movie/Test Movie (2024) - 1080p.mkv" "Test Movie 1080p" 1920x1080
video "$movie/Test Movie (2024) - 720p.mkv" "Test Movie 720p" 1280x720
image "$movie/poster.jpg" "Test Movie" 600x900 navy
image "$movie/backdrop.jpg" "Test Movie" 1920x1080 darkslateblue

movie2="$root/Movies/Second Movie (2025)"
video "$movie2/Second Movie (2025).mkv" "Second Movie" 1280x720
srt "Second Movie (external)" > "$movie2/Second Movie (2025).en.srt"
image "$movie2/poster.jpg" "Second Movie" 600x900 darkred

show="$root/Shows/Test Show"
image "$show/poster.jpg" "Test Show" 600x900 darkgreen
image "$show/backdrop.jpg" "Test Show" 1920x1080 darkolivegreen
for s in 1 2; do
  n=$([[ $s == 1 ]] && echo 3 || echo 2)
  image "$show/Season 0$s/poster.jpg" "Season $s" 600x900 seagreen
  for e in $(seq 1 "$n"); do
    video "$show/Season 0$s/Test Show S0${s}E0${e}.mkv" "Test Show S0${s}E0${e}" 1280x720
  done
done

echo "media ready in $root"
