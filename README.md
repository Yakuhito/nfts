# nfts

NFT S(napshotter) - track Chia NFT / DID / address state in a local SQLite snapshot, and generate / independently confirm the XCHandles **Base Premine**.

## What this produces

`premine generate` writes a **Base Premine** CSV only: Handles derived from viable CNS and NamesDAO Legacy Registrations at the Migration Cutoff.

This output does **not** include Contribution Premine (contributor) allocations. Those are maintained separately and merged later into the public `premine.csv` consumed by the launch landing. Do not treat a Base Premine CSV as the full published Premine.

Columns:

```text
handle,recipient,expiration,allocation_type,allocation_explanation
```

- `allocation_type` is `cns` or `namesdao`
- `allocation_explanation` is the MintGarden NFT page for the winning registration (`https://mintgarden.io/nfts/{nft_id}`)
- `expiration` is an integer UNIX timestamp (UTC)

A companion warnings CSV is also written:

```text
reason,source,nft_id,original_name,metadata_urls
```

## Tracked sources

Base Premine generation needs a fresh local snapshot of:

- **CNS** creator address: `xch1zdfcemh4cvcglzx03qu0czlaurt800agghz86c0m5uez4p30dvls8zjc8l`
- **NamesDAO** DID: `did:chia:13myvry7hmp6nwpa00lqexczka652xkyujyjsecplge8c65rtdl4qd0yya7`

MintGarden collection pages (used only by `premine confirm`):

- CNS: https://mintgarden.io/collections/chia-name-service-col10r992w4cvasaxjs7ldc0n5hlhl5dklc3x3l2tp405ra6adzczqksnw49f2
- NamesDAO: https://mintgarden.io/collections/.xch-namesdao-names-col1u9pemm2avjcz8t9emhga4vys5knugsfnctpkk2jyx05jc8d6ch2swe4qvm

## Fresh snapshot (required for a trustworthy Base Premine)

Delete any previous working database, then rebuild from the tracked sources. Do not reuse an old `nfts.db` for a cutoff run.

```bash
rm -f nfts.db nfts.db-shm nfts.db-wal

cargo run -- --db nfts.db add \
  'xch1zdfcemh4cvcglzx03qu0czlaurt800agghz86c0m5uez4p30dvls8zjc8l,did:chia:13myvry7hmp6nwpa00lqexczka652xkyujyjsecplge8c65rtdl4qd0yya7'

cargo run -- --db nfts.db sync
```

Sync can take a long time (hours) on a full CNS + NamesDAO rebuild.

## Generate the Base Premine

```bash
cargo run -- --db nfts.db premine generate \
  --output base-premine.csv \
  --warnings premine-warnings.csv
```

Behavior:

- Hydrates missing off-chain metadata (hash-verified); fails without writing outputs if any required metadata remains unavailable
- Applies CNS-first selection, succession, collision, burn-recipient, and expiration rules
- Writes both CSVs atomically (a fatal failure does not replace prior good outputs)
- Before the Migration Cutoff (`2026-07-20 09:00:00 UTC`), prints a prominent warning and continues against the latest available chain tip for **rehearsal only** — do not publish those outputs

If `premine generate` fails because off-chain CNS metadata URLs are unreachable (for example timed-out Pawket/`storage.pawket.app` hosts), recover hash-verified bytes with:

```bash
cargo run -- --db nfts.db premine mintgarden-cns-hydrate
# optional: limit to a file of nft1... ids, one per line
cargo run -- --db nfts.db premine mintgarden-cns-hydrate --nfts-file missing-nfts.txt
```

`mintgarden-cns-hydrate` is CNS-only. It rebuilds Pawket’s wire format from MintGarden’s parsed `metadata_json` (indent-2 JSON with CRLF and Pawket’s fixed key order) and **only** caches the result when SHA-256 matches the on-chain metadata hash. Fallbacks: `GET /nfts/{id}/metadata`, then on-chain metadata URLs / public IPFS gateways (still hash-asserted).

## Confirm independently (MintGarden)

```bash
cargo run -- premine confirm base-premine.csv
```

Confirmation reconstructs the expected Base Premine from MintGarden (`/collections/{id}/nfts/ids` plus per-NFT detail/`metadata_json`) and exhaustively compares it to the CSV. It never modifies the input file. Exit nonzero on any mismatch.

Rows whose recipient is the burn/null address (`xch1qqqq…m6ks6e8mvy`) are ignored on both sides — they are dropped from the published premine by `build-premine.sh`.

Pre-cutoff confirmation against MintGarden’s latest available state is rehearsal evidence only, not final cutoff confirmation. Recipients use MintGarden’s current NFT owners; a transfer after the Migration Cutoff can therefore surface as a mismatch.

## Final cutoff run (at or after 2026-07-20 09:00:00 UTC)

Repeat the full fresh-snapshot workflow; discard any pre-cutoff rehearsal database and CSVs.

```bash
rm -f nfts.db nfts.db-shm nfts.db-wal

cargo run -- --db nfts.db add \
  'xch1zdfcemh4cvcglzx03qu0czlaurt800agghz86c0m5uez4p30dvls8zjc8l,did:chia:13myvry7hmp6nwpa00lqexczka652xkyujyjsecplge8c65rtdl4qd0yya7'

cargo run -- --db nfts.db sync

cargo run -- --db nfts.db premine generate \
  --output base-premine.csv \
  --warnings premine-warnings.csv

cargo run -- premine confirm base-premine.csv
```

Only after `premine confirm` succeeds against the Migration Cutoff should the Base Premine be treated as the live artifact. Contributor allocations are still out of scope for these commands.

## Other commands

```bash
cargo run -- --db nfts.db list
cargo run -- --db nfts.db query <nft1...>
cargo run -- --db nfts.db premine mintgarden-cns-hydrate
```

## Note on MintGarden collection indexes

`GET /collections/{id}/nfts/ids` can under-count relative to a full local sync. Confirm backfills any NFT ids referenced by the CSV that are missing from those indexes (then loads each NFT’s detail page, which carries the authoritative `metadata_json`). NamesDAO collection-list `metadata` blobs often use the ineligible `___…` alias form — confirm does not trust those; it uses detail `metadata_json` instead.
