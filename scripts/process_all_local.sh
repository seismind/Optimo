#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/process_all_local.sh [dir]
# Finds images in the directory and calls `cargo run -- <files...>` (requires local tesseract installed)

DIR="${1:-.}"

mapfile -t files < <(find "$DIR" -maxdepth 1 -type f \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.tif' -o -iname '*.tiff' \) -printf '%p\n')

if [ ${#files[@]} -eq 0 ]; then
  echo "No image files found in $DIR"
  exit 1
fi

# Run locally (cargo run) passing all files
cargo run -- ${files[@]}
