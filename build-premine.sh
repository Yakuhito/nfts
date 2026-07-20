#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

{
  cat contributor-premine.csv
  echo
  # Skip header; base rows follow contributor rows.
  tail -n +2 base-premine.csv
} > premine.csv

echo "Wrote $(wc -l < premine.csv) lines to premine.csv"
