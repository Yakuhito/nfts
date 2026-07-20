#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Contribution Premine Expiration: 2027-08-20 09:00:00 UTC (launch + 1 year)
EXTENSION_FLOOR=1818752400
# Base premine expiration cap: launch + 1 year + 6 months + 122 days
# = 2028-06-21 09:00:00 UTC
BASE_EXPIRATION_CAP=1845190800

{
  cat contributor-premine.csv
  echo
  # Skip header; base rows follow contributor rows.
  # Cap base-premine expirations at BASE_EXPIRATION_CAP (keep if lower).
  awk -F',' -v OFS=',' -v cap="$BASE_EXPIRATION_CAP" '
    NR == 1 { next }
    {
      gsub(/\r/, "", $3)
      if ($3 + 0 > cap + 0) $3 = cap
      print
    }
  ' base-premine.csv
} > premine.csv.tmp

# Apply contributor extensions: for each listed handle, set expiration to
# max(current expiration, EXTENSION_FLOOR) and replace allocation_explanation.
awk -F',' -v OFS=',' -v floor="$EXTENSION_FLOOR" '
  NR == FNR {
    if (FNR == 1) next
    gsub(/\r/, "", $1)
    gsub(/\r/, "", $2)
    if ($1 != "") ext[$1] = $2
    next
  }
  FNR == 1 { print; next }
  {
    gsub(/\r/, "", $1)
    if ($1 in ext) {
      if ($3 + 0 < floor + 0) $3 = floor
      $5 = ext[$1]
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
