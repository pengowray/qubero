#!/usr/bin/env python3
"""Write the small JPEGs `crates/core/tests/jpeg_scan_real.rs` reads from
`crates/core/tests/fixtures/jpeg/`, for the layouts the sample collection has
no file of.

- `pillow-422-45x29.jpg`: 4:2:2, by Pillow.
- `pillow-420-restart-45x29.jpg`: 4:2:0 with a restart marker after every MCU,
  by Pillow.
- `three-scans-45x29.jpg`: the 4:2:0 picture again, written as three scans of
  one channel each, which is legal baseline and which Pillow cannot write. The
  coefficients are the ones `jpeg_reference.py` read out of the 4:2:0 file,
  coded again here. The second and third scans redefine Huffman table 0 before
  they start, so a decoder that reads a table by id and takes the first
  definition rather than the latest gets them wrong. The Y scan has a restart
  marker every five blocks, which in a scan of one channel counts blocks.

Pillow decodes the three-scan file to the same pixels as the file it came
from, which is the check that it is a JPEG and not only something this
repository's two decoders agree on.

    python3 tools/jpeg_fixtures.py crates/core/tests/fixtures/jpeg
"""

import io
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
import jpeg_reference as ref  # noqa: E402


def picture():
    x = np.linspace(0, 255, 45)
    y = np.linspace(0, 255, 29)
    r = np.add.outer(y, x) / 2
    g = np.abs(np.subtract.outer(y, x))
    b = np.sin(np.add.outer(y * 0.2, x * 0.3)) * 127 + 128
    rng = np.random.default_rng(7)
    arr = np.stack([r, g, b], -1) + rng.normal(0, 18, (29, 45, 3))
    return Image.fromarray(np.clip(arr, 0, 255).astype("uint8"))


def seg(marker, body):
    return struct.pack(">BBH", 0xFF, marker, len(body) + 2) + body


class Bits:
    def __init__(self):
        self.out = bytearray()
        self.acc = 0
        self.n = 0

    def put(self, value, n):
        for i in range(n - 1, -1, -1):
            self.acc = self.acc << 1 | (value >> i) & 1
            self.n += 1
            if self.n == 8:
                self.out.append(self.acc)
                if self.acc == 0xFF:
                    self.out.append(0)
                self.acc = self.n = 0

    def flush(self):
        if self.n:
            self.put((1 << (8 - self.n)) - 1, 8 - self.n)


def codes(counts, symbols):
    table = {}
    code = 0
    k = 0
    for length in range(1, 17):
        for _ in range(counts[length - 1]):
            table[symbols[k]] = (code, length)
            k += 1
            code += 1
        code <<= 1
    return table


def magnitude(v):
    size = abs(v).bit_length()
    return size, (v if v >= 0 else v + (1 << size) - 1)


def encode_block(bits, zz, pred, dc, ac):
    size, v = magnitude(zz[0] - pred)
    bits.put(*dc[size])
    bits.put(v, size)
    run = 0
    last = max([k for k in range(1, 64) if zz[k]] or [0])
    for k in range(1, last + 1):
        if zz[k] == 0:
            run += 1
            continue
        while run > 15:
            bits.put(*ac[0xF0])
            run -= 16
        size, v = magnitude(zz[k])
        bits.put(*ac[run << 4 | size])
        bits.put(v, size)
        run = 0
    if last < 63:
        bits.put(*ac[0x00])
    return zz[0]


def dht_tables(data):
    """Every Huffman table in the file, as (class, id) -> (counts, symbols)."""
    tables = {}
    for marker, _, body, _ in ref.segments(data, 0):
        if marker == 0xC4:
            i = 0
            while i < len(body):
                counts = list(body[i + 1:i + 17])
                syms = list(body[i + 17:i + 17 + sum(counts)])
                tables[(body[i] >> 4, body[i] & 15)] = (counts, syms)
                i += 17 + sum(counts)
    return tables


def three_scans(source):
    frame, scans = ref.read(source)
    (scan,) = scans
    tables = dht_tables(source)
    comps = frame["components"]
    hmax = max(c["h"] for c in comps)
    vmax = max(c["v"] for c in comps)
    blocks = {(m, bx, by): natural for (m, bx, by, natural) in scan["blocks"]}
    out = bytearray(b"\xff\xd8")
    for marker, _, body, _ in ref.segments(source, 0):
        if marker in (0xE0, 0xDB, 0xC0):
            out += seg(marker, body)
    for m, c in enumerate(comps):
        # The Y scan reads with the luminance tables as ids 0; the other two
        # read with the chrominance tables, redefined as ids 0 before them.
        which = 0 if m == 0 else 1
        dcc, dcs = tables[(0, which)]
        acc, acs = tables[(1, which)]
        if m < 2:
            out += seg(0xC4, bytes([0x00] + dcc + dcs + [0x10] + acc + acs))
        restart = 5 if m == 0 else 0
        out += seg(0xDD, struct.pack(">H", restart))
        out += seg(0xDA, bytes([1, c["id"], 0x00, 0, 63, 0]))
        dc, ac = codes(dcc, dcs), codes(acc, acs)
        cw = -(-frame["width"] * c["h"] // hmax)
        ch = -(-frame["height"] * c["v"] // vmax)
        across, down = -(-cw // 8), -(-ch // 8)
        bits = Bits()
        pred = 0
        n = 0
        for by in range(down):
            for bx in range(across):
                if restart and n and n % restart == 0:
                    bits.flush()
                    bits.out += bytes([0xFF, 0xD0 + (n // restart - 1) % 8])
                    pred = 0
                natural = blocks[(m, bx, by)]
                zz = [natural[ref.ZIGZAG[k]] for k in range(64)]
                pred = encode_block(bits, zz, pred, dc, ac)
                n += 1
        bits.flush()
        out += bits.out
    out += b"\xff\xd9"
    return bytes(out)


def main():
    where = sys.argv[1]
    os.makedirs(where, exist_ok=True)
    im = picture()
    files = {}
    for name, kw in [
        ("pillow-422-45x29.jpg", dict(quality=90, subsampling=1)),
        ("pillow-420-restart-45x29.jpg", dict(quality=80, subsampling=2, restart_marker_blocks=1)),
    ]:
        buf = io.BytesIO()
        im.save(buf, "JPEG", **kw)
        files[name] = buf.getvalue()
    files["three-scans-45x29.jpg"] = three_scans(files["pillow-420-restart-45x29.jpg"])
    a = np.asarray(Image.open(io.BytesIO(files["pillow-420-restart-45x29.jpg"])))
    b = np.asarray(Image.open(io.BytesIO(files["three-scans-45x29.jpg"])))
    assert (a == b).all(), "Pillow reads the three-scan file differently"
    for name, data in files.items():
        with open(os.path.join(where, name), "wb") as f:
            f.write(data)
        print(name, len(data))


if __name__ == "__main__":
    main()
