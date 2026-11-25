"""Write what independent readers make of radio telescope files, for the
templates to be checked against.

    python3 -I tools/radio_reference.py OUT.tsv [--root DIR] FILE [FILE ...]

One line a fact, four columns separated by tabs: what kind of fact, the file,
where in it, and the value. A file is named by its path under `--root` when
that is given, and by its own name when it is not. `crates/core/tests/radio_real.rs`
reads each fact through a template and has to say the same.

Which reader a file gets is told from its name:

- `.sf`, PSRFITS search mode. The cards are astropy's. The rows are read with
  numpy at `data offset + row * NAXIS1`, and a row's samples are unpacked with
  astropy's own bit-column unpacking for 1-bit data (`_unwrapx`, top bit
  first) and with shifts for wider samples. astropy's table reader is not
  used for the rows: it sizes a `393216X` column by its `TDIMn` card as
  393,216 bytes, against the 49,152 the column has, and refuses the table.
    - `card`, at `HDU,KEYWORD`: the card's value as text.
    - `rows`, at `HDU`: how many rows the table has.
    - `float`, at `HDU,row,column,index`: one number of a float column.
    - `sample`, at `HDU,row,index`: one sample of the row's DATA cell, counted
      from the first sample of the row.
    - `sum`, at `HDU,row,first,count`: the sum of `count` samples from `first`.
- `.fil`, a SIGPROC filterbank. Read with numpy and `struct`, from the
  header layout sigproc documents (keyword strings, each a little-endian u32
  length and its bytes, and a value whose type the keyword fixes). No third
  party filterbank reader was at hand. Where an `.h5` of the same name is
  beside it, the HDF5 copy Breakthrough Listen writes of the same header, its
  attributes are read with h5py as a second reference.
    - `key`, at `index,keyword`: the header record at that index, and its
      value as text. A string's value is its text; a mark has none.
    - `h5key`, at `keyword`: the value of that keyword as the HDF5 copy's
      attribute says it.
    - `data`, at `samples`: where the samples start, how many values a time
      sample has, and how many whole time samples there are, separated by
      commas.
    - `value`, at `index`: one value of the samples, counted from the first.
    - `sum`, at `first,count`: the sum of `count` values from `first`.
- `.raw`, GUPPI raw voltages. Read with numpy, from the GUPPI block layout:
  80-byte cards to an `END` card, the header padded to 512 bytes when
  `DIRECTIO` is 1, then `BLOCSIZE` bytes, channel by channel, each time
  sample `NPOL` values of `NBITS`, the first value in the top bits of a byte.
    - `block`, at `block`: where the block starts, how many cards its header
      has with `END`, where its data starts and how many bytes of data the
      file has for it, separated by commas.
    - `card`, at `block,KEYWORD`: a card's value as text, quotes taken off.
    - `sample`, at `block,channel,index`: one value of a channel, counted
      from the channel's first.
    - `sum`, at `block,channel,first,count`: the sum of `count` values.
- `.h5`, a filterbank in HDF5. Read with h5py and hdf5plugin.
    - `filter`, at the dataset: its filters as `id:client data`, one per
      filter, separated by `;`.
    - `chunk`, at `dataset,index`: where the chunk is in the file, how many
      bytes it is there, how many it unpacks to, the 64-bit FNV-1a hash of
      the unpacked bytes in hex, and its first four elements, separated by
      commas.
"""

import argparse
import os
import sys

import numpy as np

sys.dont_write_bytecode = True


def fnv1a(data):
    """The 64-bit FNV-1a hash, which the test can work out without a crate."""
    h = 0xCBF29CE484222325
    for b in bytes(data):
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def spread(n, most):
    """Up to `most` indexes of `n`, the two ends always among them."""
    if n <= most:
        return list(range(n))
    picks = {0, 1, n - 2, n - 1}
    rng = np.random.default_rng(n)
    picks.update(int(i) for i in rng.integers(0, n, most - len(picks)))
    return sorted(picks)


# ---------------------------------------------------------------- PSRFITS

SEARCH_CARDS = ["NBITS", "NCHAN", "NPOL", "NSBLK", "TBIN", "CHAN_BW", "NAXIS1", "NAXIS2", "POL_TYPE"]
PRIMARY_CARDS = ["FITSTYPE", "OBS_MODE", "BACKEND", "OBSFREQ", "OBSBW", "OBSNCHAN", "IBEAM", "TELESCOP"]


def psrfits(path, name, out):
    from astropy.io import fits
    from astropy.io.fits.column import _unwrapx

    with fits.open(path, memmap=True) as hdus:
        for key in PRIMARY_CARDS:
            if key in hdus[0].header:
                out("card", name, f"0,{key}", str(hdus[0].header[key]).strip())
        index = [h.name for h in hdus].index("SUBINT")
        sub = hdus[index]
        head = sub.header
        for key in SEARCH_CARDS:
            if key in head:
                out("card", name, f"{index},{key}", str(head[key]).strip())
        offset, width, rows = sub._data_offset, head["NAXIS1"], head["NAXIS2"]
        nbits, per_sample = head["NBITS"], head["NCHAN"] * head["NPOL"]
        per_row = head["NSBLK"] * per_sample
        columns = [(head[f"TTYPE{i}"].strip(), head[f"TFORM{i}"].strip()) for i in range(1, head["TFIELDS"] + 1)]
    out("rows", name, str(index), rows)
    data_bytes = per_row * nbits // 8
    table = np.memmap(path, dtype=np.uint8, mode="r", offset=offset, shape=(rows, width))
    # Where each column starts in a row, from the TFORMn cards.
    widths = {"D": 8, "E": 4, "B": 1, "I": 2, "J": 4, "K": 8, "A": 1, "L": 1}
    starts, at = {}, 0
    for i, (ttype, form) in enumerate(columns):
        digits = "".join(c for c in form if c.isdigit()) or "1"
        code = form[len(digits) if form[0].isdigit() else 0]
        size = (int(digits) + 7) // 8 if code == "X" else int(digits) * widths[code]
        starts[ttype] = (i, at, code, int(digits))
        at += size
    assert at == width, (at, width)
    assert starts["DATA"][1] + data_bytes == width
    for row in spread(rows, 12):
        raw = np.array(table[row])
        for ttype in ("TSUBINT", "OFFS_SUB", "DAT_FREQ", "DAT_WTS", "DAT_SCL"):
            col, start, code, repeat = starts[ttype]
            kind = ">f8" if code == "D" else ">f4"
            values = raw[start:start + repeat * widths[code]].view(kind)
            for i in sorted({0, repeat - 1}):
                out("float", name, f"{index},{row},{col},{i}", repr(float(values[i])))
        packed = raw[starts["DATA"][1]:]
        if nbits == 1:
            bits = np.zeros((1, per_row), dtype=bool)
            _unwrapx(packed.reshape(1, -1), bits, per_row)
            samples = bits[0].astype(np.uint8)
        elif nbits == 8:
            samples = packed
        else:
            per_byte = 8 // nbits
            shifts = np.array([8 - nbits * (i + 1) for i in range(per_byte)], dtype=np.uint8)
            samples = ((packed[:, None] >> shifts) & ((1 << nbits) - 1)).reshape(-1)
        assert len(samples) == per_row
        for i in spread(per_row, 160):
            out("sample", name, f"{index},{row},{i}", int(samples[i]))
        # The whole of the first and the last time sample, and a run across a
        # byte boundary in the middle.
        for first, count in ((0, per_sample), (per_row - per_sample, per_sample), (per_row // 2 - 3, 64)):
            out("sum", name, f"{index},{row},{first},{count}", int(samples[first:first + count].sum(dtype=np.int64)))


# ------------------------------------------------------------------- HDF5

def h5(path, name, out):
    import h5py
    import hdf5plugin  # noqa: F401, registers the bitshuffle filter

    with h5py.File(path, "r") as f:
        d = f["data"]
        plist = d.id.get_create_plist()
        filters = [plist.get_filter(i) for i in range(plist.get_nfilters())]
        out("filter", name, "data", ";".join(f"{fid}:{','.join(str(c) for c in cd)}" for fid, _, cd, _ in filters))
        for i in spread(d.id.get_num_chunks(), 16):
            info = d.id.get_chunk_info(i)
            elements = d[tuple(slice(o, o + c) for o, c in zip(info.chunk_offset, d.chunks))]
            # A chunk at the edge of the dataset is stored whole, and h5py
            # does not say what the part past the edge holds.
            if elements.shape != d.chunks:
                continue
            unpacked = elements.tobytes()
            first = ",".join(repr(x.item()) for x in elements.reshape(-1)[:4])
            out("chunk", name, f"data,{i}", f"{info.byte_offset},{info.size},{len(unpacked)},{fnv1a(unpacked):016x},{first}")


# ----------------------------------------------------------------- SIGPROC

FIL_INTS = {"telescope_id", "machine_id", "data_type", "nchans", "nbits", "nifs", "nbeams", "ibeam", "nsamples",
            "barycentric", "pulsarcentric", "nbins"}
FIL_DOUBLES = {"tstart", "tsamp", "fch1", "foff", "src_raj", "src_dej", "az_start", "za_start", "refdm", "period",
               "fchannel"}
FIL_STRINGS = {"source_name", "rawdatafile"}
FIL_MARKS = {"HEADER_START", "HEADER_END", "FREQUENCY_START", "FREQUENCY_END"}


def fil_header(buf):
    """The header's records as (keyword, value), and where the samples start."""
    import struct

    at, records = 0, []

    def string():
        nonlocal at
        (n,) = struct.unpack_from("<I", buf, at)
        text = bytes(buf[at + 4:at + 4 + n]).decode("ascii")
        at += 4 + n
        return text

    while True:
        key = string()
        if key in FIL_MARKS:
            value = None
        elif key in FIL_INTS:
            (value,) = struct.unpack_from("<i", buf, at)
            at += 4
        elif key in FIL_DOUBLES:
            (value,) = struct.unpack_from("<d", buf, at)
            at += 8
        elif key in FIL_STRINGS:
            value = string()
        elif key == "signed":
            value = buf[at]
            at += 1
        else:
            sys.exit(f"unknown keyword {key!r}")
        records.append((key, value))
        if key == "HEADER_END":
            return records, at


def fil(path, name, out):
    raw = np.fromfile(path, dtype=np.uint8)
    records, start = fil_header(raw)
    for i, (key, value) in enumerate(records):
        out("key", name, f"{i},{key}", "" if value is None else repr(value) if isinstance(value, float) else value)
    head = dict(records)
    per_sample = head["nchans"] * head.get("nifs", 1)
    nbits = head["nbits"]
    kind = {32: "<f4", 16: "<u2", 8: "<i1" if head.get("signed") == 1 else "<u1"}[nbits]
    row_bytes = per_sample * nbits // 8
    rows = (len(raw) - start) // row_bytes
    values = raw[start:start + rows * row_bytes].view(kind)
    out("data", name, "samples", f"{start},{per_sample},{rows}")
    for i in spread(len(values), 200):
        v = values[i].item()
        out("value", name, str(i), repr(v) if isinstance(v, float) else v)
    for first, count in ((0, min(per_sample, 4096, len(values))), (max(0, len(values) - 4096), min(len(values), 4096))):
        if count:
            total = float(np.sum(values[first:first + count], dtype=np.float64)) if kind == "<f4" else int(values[first:first + count].sum(dtype=np.int64))
            out("sum", name, f"{first},{count}", repr(total) if isinstance(total, float) else total)
    twin = os.path.splitext(path)[0] + ".h5"
    if os.path.exists(twin):
        import h5py

        with h5py.File(twin, "r") as f:
            for key, value in sorted(f["data"].attrs.items()):
                if key == "DIMENSION_LABELS":
                    continue
                value = value.item() if hasattr(value, "item") else value
                out("h5key", name, key, repr(value) if isinstance(value, float) else value)


# ------------------------------------------------------------------- GUPPI

def guppi(path, name, out):
    raw = np.memmap(path, dtype=np.uint8, mode="r")
    at, index = 0, 0
    while at < len(raw):
        cards, n = {}, 0
        while True:
            card = bytes(raw[at + 80 * n:at + 80 * (n + 1)]).decode("ascii")
            n += 1
            if card[:8] == "END     ":
                break
            if card[8:10] == "= ":
                cards[card[:8].strip()] = card[10:].split("/")[0].strip().strip("'").strip()
        header = 80 * n
        if int(cards.get("DIRECTIO", "0")) != 0:
            header = -(-header // 512) * 512
        blocsize = int(cards["BLOCSIZE"])
        data = raw[at + header:at + header + blocsize]
        out("block", name, str(index), f"{at},{n},{at + header},{len(data)}")
        for key in ("BLOCSIZE", "OBSNCHAN", "NPOL", "NBITS", "TBIN", "CHAN_BW", "OBSFREQ", "DIRECTIO", "PKTIDX", "SRC_NAME"):
            if key in cards:
                out("card", name, f"{index},{key}", cards[key])
        nchan, nbits = int(cards["OBSNCHAN"]), int(cards["NBITS"])
        per_channel = blocsize // nchan
        for c in sorted({0, 1, nchan // 2, nchan - 1}):
            chunk = np.array(data[c * per_channel:(c + 1) * per_channel])
            if nbits == 8:
                values = chunk.view(np.int8).astype(np.int64)
            else:
                per_byte = 8 // nbits
                shifts = np.array([8 - nbits * (i + 1) for i in range(per_byte)], dtype=np.uint8)
                values = ((chunk[:, None] >> shifts) & ((1 << nbits) - 1)).reshape(-1).astype(np.int64)
            if len(values) == 0:
                continue
            for i in spread(len(values), 40):
                out("sample", name, f"{index},{c},{i}", int(values[i]))
            for first, count in ((0, min(len(values), 4096)), (len(values) // 2, min(len(values) - len(values) // 2, 4096))):
                out("sum", name, f"{index},{c},{first},{count}", int(values[first:first + count].sum()))
        pad = 0
        if int(cards.get("DIRECTIO", "0")) != 0:
            pad = -blocsize % 512
        at += header + blocsize + pad
        index += 1


# ------------------------------------------------------------------- main

READERS = {".sf": psrfits, ".h5": h5, ".fil": fil, ".raw": guppi}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("out")
    ap.add_argument("--root")
    ap.add_argument("files", nargs="+")
    args = ap.parse_args()
    lines = ["# kind\tfile\twhere\tvalue"]

    def out(kind, name, where, value):
        lines.append(f"{kind}\t{name}\t{where}\t{value}")

    for path in args.files:
        name = os.path.relpath(path, args.root).replace(os.sep, "/") if args.root else os.path.basename(path)
        reader = READERS.get(os.path.splitext(path)[1])
        if reader is None:
            sys.exit(f"{path}: no reader for this kind of file")
        before = len(lines)
        reader(path, name, out)
        print(f"{name}: {len(lines) - before} facts")
    with open(args.out, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
