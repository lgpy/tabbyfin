#!/usr/bin/env bash
# Generates a small synthetic media library under dev/.data/media.
# Covers: a movie with three versions (one 4K HEVC 10-bit, for hardware decoding), a series with two seasons, multiple audio
# languages, embedded and external subtitles, chapters (Intro, Episode, Credits), and local artwork.
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)/.data/media"
dur="${DURATION:-60}"

srt() {
  local title="$1"
  printf '1\n00:00:01,000 --> 00:00:05,000\n%s\n\n2\n00:00:10,000 --> 00:00:15,000\nTen seconds in.\n\n3\n00:00:30,000 --> 00:00:35,000\nHalfway there.\n' "$title"
}

# Chapters: Intro for the first 10 s, Credits for the last 15 s.
chapters() {
  local ms=$((dur * 1000))
  printf ';FFMETADATA1\n'
  printf '[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=10000\ntitle=Intro\n'
  printf '[CHAPTER]\nTIMEBASE=1/1000\nSTART=10000\nEND=%d\ntitle=Episode\n' $((ms - 15000))
  printf '[CHAPTER]\nTIMEBASE=1/1000\nSTART=%d\nEND=%d\ntitle=Credits\n' $((ms - 15000)) "$ms"
}

# add_chapters <file.mkv>: remuxes a file made before chapters were added.
add_chapters() {
  local file="$1" meta tmp
  [[ -n "$(ffprobe -loglevel error -show_chapters -of csv "$file")" ]] && return
  meta="$(mktemp)"
  tmp="${file%.mkv}.tmp.mkv"
  chapters > "$meta"
  ffmpeg -loglevel error -y -i "$file" -i "$meta" -map 0 -map_chapters 1 -c copy "$tmp"
  mv "$tmp" "$file"
  rm -f "$meta"
}

# video <out.mkv> <title> <size> [hevc10]
video() {
  local out="$1" title="$2" size="$3" sub
  local codec=(-c:v libx264 -preset ultrafast -crf 32 -pix_fmt yuv420p)
  [[ "${4:-}" == hevc10 ]] && codec=(-c:v libx265 -preset ultrafast -crf 32 -pix_fmt yuv420p10le -x265-params log-level=error)
  [[ -f "$out" ]] && { add_chapters "$out"; return; }
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
    "${codec[@]}" \
    -c:a aac -b:a 64k -c:s srt \
    -metadata title="$title" \
    -metadata:s:a:0 language=eng -metadata:s:a:0 title="English" \
    -metadata:s:a:1 language=jpn -metadata:s:a:1 title="Japanese" \
    -metadata:s:s:0 language=eng \
    "$out"
  rm -f "$sub"
  add_chapters "$out"
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
video "$movie/Test Movie (2024) - 4K HEVC.mkv" "Test Movie 4K HEVC 10-bit" 3840x2160 hevc10
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
