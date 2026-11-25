"""Which end of a byte a PSRFITS search-mode file puts its first channel in.

    python3 -I tools/psrfits_bit_order.py FILE.sf [FILE.sf ...]

Channels next to each other in frequency are correlated: they see the same
interference and the same changes in the receiver. A byte of 1-bit samples
holds eight channels and a byte of 2-bit samples four, so the last channel of
one byte and the first of the next are neighbours too, but only when the
samples are unpacked in the order they were packed. Unpacked the other way,
those two are channels far apart and their correlation drops to nothing.

For each file this unpacks the first 20 rows of the `SUBINT` table's `DATA`
column both ways and prints the median correlation of neighbouring channels,
for pairs inside one byte and for pairs across a byte boundary. The order
whose two numbers agree is the one the file was written in.

On 2026-11-25, for a Parkes multibeam 1-bit file (`PH0037_01711.sf`) and an
HTRU 2-bit file (`bpsr111206_072835_beam10.sf`):

    top bits first: inside 0.1287, across 0.1544  (1-bit)
    low bits first: inside 0.1287, across 0.0096
    top bits first: inside 0.0829, across 0.0827  (2-bit)
    low bits first: inside 0.0829, across 0.0002

So both put the first channel in the top bits of a byte.
"""

import sys

import numpy as np
from astropy.io import fits

sys.dont_write_bytecode = True


def unpack(raw, nbits, top_first):
    """Every sample of each row of `raw`, in the order given."""
    per = 8 // nbits
    shifts = [8 - nbits * (i + 1) for i in range(per)] if top_first else [nbits * i for i in range(per)]
    shifts = np.array(shifts, dtype=np.uint8)
    return ((raw[..., None] >> shifts) & ((1 << nbits) - 1)).reshape(raw.shape[0], -1)


def main():
    for path in sys.argv[1:]:
        with fits.open(path, memmap=True) as hdus:
            sub = hdus["SUBINT"]
            head = sub.header
            nbits, nchan, width, offset = head["NBITS"], head["NCHAN"] * head["NPOL"], head["NAXIS1"], sub._data_offset
            rows, nsblk = head["NAXIS2"], head["NSBLK"]
        table = np.memmap(path, dtype=np.uint8, mode="r", offset=offset, shape=(rows, width))
        # DATA is the last column, so its bytes end the row.
        data_bytes = nsblk * nchan * nbits // 8
        data = np.array(table[: min(rows, 20), width - data_bytes :]).reshape(-1, nchan * nbits // 8)
        per = 8 // nbits
        for top_first in (True, False):
            x = unpack(data, nbits, top_first).astype(np.float64)
            x -= x.mean(0)
            x /= x.std(0) + 1e-12
            c = (x[:, :-1] * x[:, 1:]).mean(0)
            inside = np.median([c[i] for i in range(nchan - 1) if (i + 1) % per != 0])
            across = np.median([c[i] for i in range(nchan - 1) if (i + 1) % per == 0])
            order = "top bits first" if top_first else "low bits first"
            print(f"{path.split('/')[-1]}: {order}: inside {inside:.4f}, across {across:.4f}")


if __name__ == "__main__":
    main()
