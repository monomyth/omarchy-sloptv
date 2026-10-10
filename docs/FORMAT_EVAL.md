# Feed format evaluation: JSON vs Parquet / MessagePack / CBOR / bincode

Measured on this box (release build, `cargo` bench crate in /tmp, not shipped) with the live feed
(594 records, 289,751 bytes as JSON) and a synthetic 5,000-record feed made by repeating the live records
(repeats make the columnar/zstd numbers look better than a real feed would).

| 594 real records | raw bytes | gzip -9 | parse time |
|---|---|---|---|
| JSON (serde_json) | 289,751 | 70,396 | 0.50 ms |
| MessagePack, named fields | 261,359 | 72,243 | 0.39 ms |
| MessagePack, array form | 213,245 | 69,202 | n/a |
| CBOR | 261,723 | 72,144 | 0.80 ms |
| bincode 1.3 | 255,725 | 71,992 | 0.25 ms |
| Parquet (zstd) | 77,291 | (already compressed) | 0.39 ms (to Arrow batches) |

5,000 records: JSON 2.46 MB / 589 KB gz / 4.4 ms; MessagePack 2.2 MB / 605 KB gz / 3.5 ms; bincode 2.2 MB / 2.3 ms;
Parquet 110 KB / 0.84 ms (synthetic duplicates inflate this).

The server already answers `Accept-Encoding: br` with about 70 KB, i.e. the same size as Parquet at 594 records.

Findings
* Size: on the wire JSON+brotli (70 KB) equals Parquet (77 KB). Raw binary row formats (MessagePack, CBOR, bincode) are only 10-25 % smaller than JSON and compress no better.
* Parse time: every option is under 1 ms at 594 records and under 10 ms at 5,000. Nothing here is visible to a person.
* Crates: Parquet needs `parquet` + `arrow` (about 2 minutes of extra compile time, +~7 MB binary in the bench, dozens of transitive crates). MessagePack/CBOR/bincode are one small crate each (`rmp-serde`, `ciborium`, `bincode` 1.3; note bincode 2/3 on crates.io differ or are stubs).
* HTTP: all are plain static files, so ETag / If-None-Match / Range behave identically. Parquet is the only one that benefits from Range (read footer + needed columns), which is pointless for a 70 KB file.
* Debuggability: JSON can be read with `curl | jq`; Parquet needs parquet-tools/duckdb; MessagePack/CBOR need a decoder.

Recommendation: keep JSON (served with brotli/gzip and ETag) as the only source. No binary source was added to slopTV because no format wins by a margin anyone could notice, and each costs tooling. Revisit Parquet only if the feed grows past roughly 50k records.
