# Bundled ImHex patterns

Verbatim copies of `.hexpat` files from
[ImHex-Patterns](https://github.com/WerWolv/ImHex-Patterns), compiled into
`qubero-core` and converted on demand. `crates/core/src/hexpat/bundled.rs` is
the table; this file is the record of which patterns are here and why.

The repository is GPL-2.0 as a whole, so almost none of it can be copied in.
These 4 files each carry a licence of their own in the header, and those are
the only licences allowed here: `MIT`, `MPL-2.0`. Nothing under the repository
licence is bundled, and neither are the include files under `includes/`, which
are GPL-2.0 too. A bundled pattern that imports one is given the
declaration-only table in `crates/core/src/hexpat/includes.rs` instead.

Every other pattern in the library is reachable from the converter panel, which
fetches the one a reader picks and keeps it for that session only.

Both this file and the table are generated. `node tools/hexpat_bundle.mjs`
rewrites them from the directory. Nothing here is edited by hand except the
reasons, which live in the script.

## What the columns say

* **Bundled** is `yes` for a pattern a reader can open a file as, and
  `import` for a file that is here because another one imports it.
* **Gaps** is how many things in the pattern the IR cannot say. The conversion
  report names every one with the line and column it came from, and the panel
  shows them. A test asserts this number, so the table cannot drift.
* **Magic** is what `#pragma magic` pins, which is the one thing that may claim
  a dropped file. A magic measured back from the end of the file is not one
  leading bytes can be matched against, so it is not counted.
* **Reason** says why a pattern with gaps ships anyway. Every one of these has
  gaps: the ImHex pattern language has an imperative half that no template can
  hold, and the test is whether what was left behind is off the path through
  the file.

| Pattern | Licence | Bundled | Gaps | Magic | Reason |
| --- | --- | --- | --- | --- | --- |
| `bink_container` | MPL-2.0 | yes | 5 | 3 bytes at 0x0 | three bit fields cross a byte boundary packed from the low bit up, and one cursor move ends the audio track list; every frame offset is placed |
| `gltf` | MIT | yes | 4 | 4 bytes at 0x0 | the JSON chunk is decoded by an ImHex plugin rather than by the pattern, so its bytes are left unread; the header and the chunk table are placed |
| `mbr` | MPL-2.0 | yes | 1 | 2 bytes at 0x1fe | one `break` inside the partition loop; all four partition entries are placed |
| `vhd` | MPL-2.0 | yes | 4 | no magic | one cursor move to the footer and one `try`; the footer magic, read 512 bytes back from the end, and the dynamic-disc header are placed |

## Licences

Each file's licence is in its own header and again in `THIRD-PARTY-NOTICES.md`
beside the licence text and the URL it came from.
