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

## Confirm independently (MintGarden)

```bash
cargo run -- premine confirm base-premine.csv
```

Confirmation reconstructs the expected Base Premine from MintGarden collection + event APIs and exhaustively compares it to the CSV. It never modifies the input file. Exit nonzero on any mismatch.

Pre-cutoff confirmation against MintGarden’s latest available state is rehearsal evidence only, not final cutoff confirmation.

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
```
