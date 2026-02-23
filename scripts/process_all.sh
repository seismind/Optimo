#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/process_all.sh [dir]
# Scans the given directory (default: current) for common image files and
# runs the Optimo Docker image on them in a single invocation.

DIR="${1:-.}"
PWD_DIR=$(pwd)

# Collect file basenames from DIR (not recursive)
mapfile -t files < <(find "$DIR" -maxdepth 1 -type f \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.tif' -o -iname '*.tiff' \) -printf '%f\n')

if [ ${#files[@]} -eq 0 ]; then
  echo "No image files found in $DIR"
  exit 1
fi

args=()
for f in "${files[@]}"; do
  args+=("/workspace/$f")
done

# Run docker with user mapping to avoid root-owned outputs
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD_DIR/data:/app/data" -v "$PWD_DIR:/workspace" optimo "${args[@]}"
