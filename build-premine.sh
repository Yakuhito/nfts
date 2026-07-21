#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Contribution Premine Expiration: 2027-08-20 09:00:00 UTC (launch + 1 year)
EXTENSION_FLOOR=1818752400
# Launch Instant + 122 days: 2026-12-20 09:00:00 UTC (hoarder base-premine expiration)
HOARDER_EXPIRATION=1797757200
HOARDER_HANDLE_LIMIT=10
# Burn / null recipient — drop these rows from the published premine.
DEAD_ADDRESS='xch1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqm6ks6e8mvy'

: > /tmp/a

{
  # Ensure a trailing newline so the first base row does not glue onto the last contributor row.
  sed -e '$a\' contributor-premine.csv
  tail -n +2 base-premine.csv
} > premine.csv.tmp

# Drop DEAD_ADDRESS rows, then apply hoarder rule (CNS/NamesDAO only): recipients with
# more than HOARDER_HANDLE_LIMIT allocated base-premine handles expire at Launch Instant + 122 days.
# Contributor rows are never modified by this pass.
awk -F',' -v OFS=',' -v limit="$HOARDER_HANDLE_LIMIT" -v hoarder_exp="$HOARDER_EXPIRATION" \
    -v dead="$DEAD_ADDRESS" '
  NF == 0 { next }
  FNR == 1 { print; next }
  {
    gsub(/\r/, "", $0)
    if ($2 == dead) {
      dropped_dead++
      next
    }
    if ($4 == "cns" || $4 == "namesdao") {
      count[$2]++
      base_rows[++nbase] = $0
      base_recip[nbase] = $2
    } else {
      print
    }
  }
  END {
    for (i = 1; i <= nbase; i++) {
      split(base_rows[i], f, ",")
      if (count[base_recip[i]] > limit) {
        f[3] = hoarder_exp
        hoarders[base_recip[i]] = count[base_recip[i]]
      }
      out = f[1]
      for (j = 2; j <= 5; j++) out = out OFS f[j]
      print out
    }
    for (addr in hoarders) {
      printf "%s\t%d\n", addr, hoarders[addr] > "/tmp/a"
    }
    if (dropped_dead > 0) {
      printf "Dropped %d DEAD_ADDRESS row(s)\n", dropped_dead > "/dev/stderr"
    }
  }
' premine.csv.tmp > premine.csv.hoisted

# Apply contributor extensions: for each listed handle, set expiration to
# max(current expiration, EXTENSION_FLOOR) and replace allocation_explanation.
awk -F',' -v OFS=',' -v floor="$EXTENSION_FLOOR" '
  NR == FNR {
    if (FNR == 1 || NF == 0) next
    gsub(/\r/, "", $1)
    gsub(/\r/, "", $2)
    if ($1 != "") ext[$1] = $2
    next
  }
  NF == 0 { next }
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
' contributor-extensions.csv premine.csv.hoisted > premine.csv.extended

# Apply yoinks: override recipient, set allocation_type=contributor, set explanation.
# Logs: YOINKED [handle] from [details] to [details]
awk -F',' -v OFS=',' '
  NR == FNR {
    if (FNR == 1 || NF == 0) next
    gsub(/\r/, "", $1)
    gsub(/\r/, "", $2)
    gsub(/\r/, "", $3)
    if ($1 == "") next
    yoink_recip[$1] = $2
    yoink_expl[$1] = $3
    next
  }
  NF == 0 { next }
  FNR == 1 { print; next }
  {
    gsub(/\r/, "", $1)
    if ($1 in yoink_recip) {
      from = $2 "," $4 "," $5
      $2 = yoink_recip[$1]
      $4 = "contributor"
      $5 = yoink_expl[$1]
      to = $2 "," $4 "," $5
      printf "YOINKED %s from %s to %s\n", $1, from, to > "/dev/stderr"
      delete yoink_recip[$1]
      delete yoink_expl[$1]
    }
    print
  }
  END {
    for (h in yoink_recip) {
      printf "error: yoink handle not found in premine: %s\n", h > "/dev/stderr"
      missing = 1
    }
    if (missing) exit 1
  }
' yoinks.csv premine.csv.extended > premine.csv

rm -f premine.csv.tmp premine.csv.hoisted premine.csv.extended

if [[ -s /tmp/a ]]; then
  sort -k2,2nr -k1,1 /tmp/a -o /tmp/a
  echo "Wrote $(wc -l < /tmp/a) hoarder addresses to /tmp/a"
else
  echo "No hoarder addresses (wrote empty /tmp/a)"
fi

echo "Wrote $(wc -l < premine.csv) lines to premine.csv"

# Verify published CSV: header + every data row must have exactly 5 comma-separated fields
# (i.e. exactly 4 commas per line).
awk -F',' '
  NF == 0 { next }
  {
    gsub(/\r/, "", $0)
    # Count commas directly so empty trailing fields still fail clearly via NF.
    n = gsub(/,/, ",", $0)
    if (n != 4) {
      printf "error: premine.csv line %d has %d comma(s), expected 4: %s\n", NR, n, $0 > "/dev/stderr"
      bad = 1
    }
  }
  END {
    if (bad) exit 1
    print "Verified premine.csv: every line has exactly 4 commas"
  }
' premine.csv
