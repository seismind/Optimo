#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/setup_data.sh [data_dir]
# Creates data directories and adjusts ownership to the current user.

DATA_DIR="${1:-data}"
OCRYS_DIR="$DATA_DIR/ocrys"
LATEST_DIR="$OCRYS_DIR/latest"

mkdir -p "$LATEST_DIR"

uid=$(id -u)
gid=$(id -g)
chown -R "$uid:$gid" "$DATA_DIR"

echo "Created $LATEST_DIR and set ownership to $uid:$gid" 
