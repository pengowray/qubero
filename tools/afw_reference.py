"""Write what astropy reads from FITS files in the LSST Science Pipelines'
layout (afw), for the template to be checked against.

    python -I tools/afw_reference.py OUT.tsv FILE.fits [FILE.fits ...]

One line a fact, five columns: what kind of fact, the file, the HDU (counted
from 0), where in it, and the value.

- `plane`: a mask plane, from the mask HDU's `MP_<NAME> = <bit>` cards.
  Where is the name, the value the bit.
- `pixel`: a mask pixel, as the integer it is. Where is `y,x`, counted from 0.
  For each plane, the first few pixels with its bit set, and a few with none
  set. A tile-compressed mask's pixels are all in its first tile, which is
  the one a reader is shown first.
- `card`: the value of an `ARCHIVE_ID_*` card in the primary header, by its
  long keyword, and of an older `PSF_ID` card whose comment says it is an
  archive ID.
- `index`: a row of the `ARCHIVE_INDEX` table. Where is the row, the value
  its seven cells joined by `;`: id, cat.archive, cat.persistable, row0,
  nrows, name and module.
- `blob`: a cell of bytes in a table after the index. Where is `column,row`,
  the value what the bytes start with (`pickle`, `text` or `bytes`), how many
  there are and the first eight in hex.
"""

import sys
import os

import numpy as np
from astropy.io import fits

PER_PLANE = 3


def mask_hdu(hdus):
    for i, h in enumerate(hdus):
        if h.header.get("EXTTYPE", "").strip() == "MASK":
            return i
    return None


def planes(header):
    out = []
    for card in header.cards:
        if card.keyword.startswith("MP_"):
            out.append((card.keyword[3:], int(card.value)))
    return sorted(out, key=lambda p: p[1])


def pixels(data, found, region):
    """The first few pixels of each plane in `region`, and a few with no
    plane set: (y, x, value)."""
    h, w = region
    part = np.asarray(data[:h, :w]).astype(np.int64)
    chosen = []
    for _, bit in found:
        ys, xs = np.nonzero(part & (1 << bit))
        chosen.extend(zip(ys[:PER_PLANE], xs[:PER_PLANE]))
    ys, xs = np.nonzero(part == 0)
    chosen.extend(zip(ys[:PER_PLANE], xs[:PER_PLANE]))
    chosen.append((h - 1, w - 1))
    seen = []
    for y, x in chosen:
        if (int(y), int(x)) not in seen:
            seen.append((int(y), int(x)))
    return [(y, x, int(part[y, x])) for y, x in sorted(seen)]


def blob_kind(b):
    if len(b) >= 2 and b[0] == 0x80 and 2 <= b[1] <= 5:
        return "pickle"
    if b and all(c in (9, 10, 13) or 32 <= c <= 126 for c in b):
        return "text"
    return "bytes"


def read(path):
    name = os.path.basename(path)
    lines = []
    # The mask as it is written, with no unpacking, for its header and the
    # shape of its tiles; and unpacked, for its pixels.
    with fits.open(path, disable_image_compression=True) as raw, fits.open(path) as hdus:
        m = mask_hdu(raw)
        if m is not None:
            header = raw[m].header
            found = planes(header)
            for plane, bit in found:
                lines.append(("plane", name, m, plane, bit))
            data = hdus[m].data
            if header.get("ZIMAGE"):
                region = (header.get("ZTILE2", 1), header.get("ZTILE1", header["ZNAXIS1"]))
            else:
                region = data.shape
            # A mask scaled to unsigned keeps the integers it is worth.
            for y, x, v in pixels(data, found, region):
                lines.append(("pixel", name, m, f"{y},{x}", v))
        for card in raw[0].header.cards:
            if card.keyword.startswith("ARCHIVE_ID_") or (card.keyword.endswith("_ID") and "archive ID" in card.comment):
                lines.append(("card", name, 0, card.keyword, int(card.value)))
        index = None
        for i, h in enumerate(raw):
            if isinstance(h, fits.BinTableHDU) and h.header.get("EXTTYPE") == "ARCHIVE_INDEX":
                index = i
                for r, row in enumerate(h.data):
                    cells = [str(v).strip() if isinstance(v, str) else str(int(v)) for v in row]
                    lines.append(("index", name, i, r, ";".join(cells)))
        if index is not None:
            for i in range(index + 1, len(raw)):
                h = raw[i]
                if not isinstance(h, fits.BinTableHDU):
                    continue
                for c in h.columns:
                    if c.format.startswith(("1QB", "1PB")) or c.format in ("QB", "PB"):
                        for r in range(len(h.data)):
                            b = bytes(np.asarray(h.data[c.name][r], dtype=np.uint8))
                            lines.append(("blob", name, i, f"{c.name},{r}", f"{blob_kind(b)} {len(b)} {b[:8].hex()}"))
    return lines


def main():
    target, files = sys.argv[1], sys.argv[2:]
    with open(target, "w") as fd:
        fd.write("# kind\tfile\thdu\twhere\tvalue\n")
        for f in files:
            lines = read(f)
            for line in lines:
                fd.write("\t".join(map(str, line)) + "\n")
            kinds = {}
            for line in lines:
                kinds[line[0]] = kinds.get(line[0], 0) + 1
            print(os.path.basename(f), ", ".join(f"{n} {k}" for k, n in kinds.items()))


if __name__ == "__main__":
    main()
