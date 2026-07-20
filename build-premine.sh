#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Contribution Premine Expiration: 2027-08-20 09:00:00 UTC
EXTENSION_FLOOR=1818752400

{
  cat contributor-premine.csv
  echo
  # Skip header; base rows follow contributor rows.
  tail -n +2 base-premine.csv
} > premine.csv.tmp

# Apply contributor extensions: for each listed handle, set expiration to
# max(current expiration, EXTENSION_FLOOR).
awk -F',' -v OFS=',' -v floor="$EXTENSION_FLOOR" '
  NR == FNR {
    if (FNR == 1) next
    gsub(/\r/, "", $1)
    if ($1 != "") ext[$1] = 1
    next
  }
  FNR == 1 { print; next }
  {
    gsub(/\r/, "", $1)
    if ($1 in ext) {
      if ($3 + 0 < floor + 0) $3 = floor
      delete ext[$1]
    }
    print
  }
  END {
    for (h in ext) {
      printf "error: extension handle not found in premine: %s\n", h > "/dev/stderr"
      missing = 1
    }
    if (missing) exit 1
  }
' contributor-extensions.csv premine.csv.tmp > premine.csv

rm -f premine.csv.tmp

echo "Wrote $(wc -l < premine.csv) lines to premine.csv"
