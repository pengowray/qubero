#!/usr/bin/env python3
"""A baseline JPEG scan decoder written apart from Qubero's, to check it by.

Reads every scan of a sequential Huffman JPEG (SOF0, or SOF1 at 8 bits) and
prints, per scan, what `crates/core/tests/jpeg_scan_real.rs` pins:

- how many 8x8 blocks the scan holds, and a CRC-32 of their quantized
  coefficients, 64 per block in natural (row by row) order, each a
  little-endian i16, blocks in the order the scan codes them;
- where the scan's bits went: Huffman code bits, value bits, padding bits,
  stuffed zero bytes, restart markers, and anything after the last MCU. These
  have to add up to the scan's length exactly.

With --pillow it also runs the inverse DCT and holds the result against
Pillow's own decode of the file, which is libjpeg's: within 2 levels on every
sample of every full-resolution channel, since libjpeg's IDCT is an integer
one and this one is floating point. Channels stored at less than full
resolution are left out, because libjpeg smooths them as it enlarges them.

Nothing here is shared with the Rust decoder: the segments are read with
`struct`, the Huffman codes are looked up as strings of noughts and ones, and
the byte stuffing is taken out before any bit is read. Slow, and meant to be.

    python3 tools/jpeg_reference.py [--pillow] [--offset N] FILE...
"""

import json
import struct
import sys
import zlib

ZIGZAG = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5,
    12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
]


class NotBaseline(Exception):
    pass


def segments(data, start):
    """Yield (marker, offset of the marker, body) for each segment, and for a
    scan also the offset where its entropy-coded data ends."""
    at = start
    assert data[at:at + 2] == b"\xff\xd8", "no SOI"
    at += 2
    while at + 4 <= len(data):
        if data[at] != 0xFF:
            raise ValueError(f"no marker at {at:#x}")
        marker = data[at + 1]
        if marker == 0xFF:  # fill byte
            at += 1
            continue
        if marker == 0xD9:
            yield marker, at, b"", None
            return
        if 0xD0 <= marker <= 0xD7 or marker == 0x01:
            at += 2
            continue
        (length,) = struct.unpack(">H", data[at + 2:at + 4])
        body = data[at + 4:at + 2 + length]
        end = at + 2 + length
        if marker == 0xDA:
            # The entropy-coded data runs to the next marker that is neither a
            # stuffed 0xff nor a restart.
            e = end
            while e < len(data):
                if data[e] == 0xFF and e + 1 < len(data) and data[e + 1] not in (0x00, 0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7):
                    break
                if data[e] == 0xFF and e + 1 >= len(data):
                    break
                e += 1
            yield marker, at, body, (end, e)
            at = e
            continue
        yield marker, at, body, None
        at = end


def huffman(counts, symbols):
    """Code strings to symbols, by the rule in Annex C."""
    table = {}
    code = 0
    k = 0
    for length in range(1, 17):
        for _ in range(counts[length - 1]):
            table[format(code, f"0{length}b")] = symbols[k]
            k += 1
            code += 1
        code <<= 1
    return table


def extend(v, t):
    return v - (1 << t) + 1 if t and v < (1 << (t - 1)) else v


def decode_scan(data, run, frame, scan, dc_tables, ac_tables, restart):
    comps = frame["components"]
    hmax = max(c["h"] for c in comps)
    vmax = max(c["v"] for c in comps)
    width, height = frame["width"], frame["height"]
    by_id = {c["id"]: i for i, c in enumerate(comps)}
    members = [by_id[sc["id"]] for sc in scan["components"]]

    # The order the blocks are coded in: a list of (component, bx, by).
    order = []
    if len(members) == 1:
        c = comps[members[0]]
        cw = -(-width * c["h"] // hmax)
        ch = -(-height * c["v"] // vmax)
        for by in range(-(-ch // 8)):
            for bx in range(-(-cw // 8)):
                order.append([(members[0], bx, by)])
    else:
        mx = -(-width // (8 * hmax))
        my = -(-height // (8 * vmax))
        for y in range(my):
            for x in range(mx):
                mcu = []
                for m in members:
                    c = comps[m]
                    for v in range(c["v"]):
                        for h in range(c["h"]):
                            mcu.append((m, x * c["h"] + h, y * c["v"] + v))
                order.append(mcu)

    begin, end = run
    raw = data[begin:end]
    # Cut the run at its restart markers and take the stuffing out of each
    # piece before reading a bit of it.
    pieces = []
    stuffed = 0
    markers = 0
    cur = bytearray()
    i = 0
    while i < len(raw):
        b = raw[i]
        if b == 0xFF and i + 1 < len(raw):
            nxt = raw[i + 1]
            if nxt == 0x00:
                cur.append(0xFF)
                stuffed += 1
                i += 2
                continue
            if 0xD0 <= nxt <= 0xD7:
                pieces.append((bytes(cur), nxt - 0xD0))
                cur = bytearray()
                markers += 1
                i += 2
                continue
        cur.append(b)
        i += 1
    pieces.append((bytes(cur), None))

    interval = restart if restart else len(order)
    stats = dict(code_bits=0, value_bits=0, padding_bits=0, stuffed=stuffed, markers=markers, after=0)
    coefs = bytearray()
    blocks = []
    mcu_at = 0
    piece = 0
    while mcu_at < len(order):
        if piece >= len(pieces):
            raise ValueError("the scan ran out of data")
        bytes_, rst = pieces[piece]
        if piece > 0 and pieces[piece - 1][1] != (piece - 1) % 8:
            raise ValueError("restart markers out of order")
        bits = "".join(format(b, "08b") for b in bytes_)
        pos = 0
        pred = [0] * len(comps)
        for _ in range(interval):
            if mcu_at >= len(order):
                break
            for (m, bx, by) in order[mcu_at]:
                sc = scan["components"][members.index(m)]
                dct = dc_tables[sc["dc"]]
                act = ac_tables[sc["ac"]]
                zz = [0] * 64

                def code(table):
                    nonlocal pos
                    for n in range(1, 17):
                        s = bits[pos:pos + n]
                        if len(s) < n:
                            raise ValueError("a code ran past the data")
                        if s in table:
                            pos += n
                            stats["code_bits"] += n
                            return table[s]
                    raise ValueError("no such code")

                def receive(t):
                    nonlocal pos
                    if t == 0:
                        return 0
                    s = bits[pos:pos + t]
                    if len(s) < t:
                        raise ValueError("value bits ran past the data")
                    pos += t
                    stats["value_bits"] += t
                    return extend(int(s, 2), t)

                t = code(dct)
                pred[m] += receive(t)
                zz[0] = pred[m]
                k = 1
                while k < 64:
                    rs = code(act)
                    r, s = rs >> 4, rs & 15
                    if s == 0:
                        if r == 15:
                            k += 16
                            continue
                        break
                    k += r
                    if k > 63:
                        raise ValueError("a coefficient past 63")
                    zz[k] = receive(s)
                    k += 1
                natural = [0] * 64
                for j in range(64):
                    natural[ZIGZAG[j]] = zz[j]
                coefs += struct.pack("<64h", *natural)
                blocks.append((m, bx, by, natural))
            mcu_at += 1
        # What is left of the last byte is padding.
        stats["padding_bits"] += (-pos) % 8
        used = (pos + 7) // 8
        if used < len(bytes_):
            if rst is None and mcu_at >= len(order):
                stats["after"] += len(bytes_) - used
            else:
                raise ValueError(f"{len(bytes_) - used} bytes left in an interval")
        piece += 1
    if piece < len(pieces):
        # Restart markers after the last MCU, and whatever follows them.
        for bytes_, _ in pieces[piece:]:
            stats["after"] += len(bytes_)
    total = stats["code_bits"] + stats["value_bits"] + stats["padding_bits"] + 8 * stats["stuffed"] + 16 * stats["markers"] + 8 * stats["after"]
    assert total == 8 * len(raw), f"the bits come to {total}, not {8 * len(raw)}"
    return coefs, blocks, stats


def read(data, start=0):
    frame = None
    dc_tables, ac_tables, quant = {}, {}, {}
    restart = 0
    scans = []
    for marker, at, body, run in segments(data, start):
        if marker in (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF):
            p, h, w, n = struct.unpack(">BHHB", body[:6])
            comps = []
            for i in range(n):
                cid, hv, tq = body[6 + 3 * i:9 + 3 * i]
                comps.append(dict(id=cid, h=hv >> 4, v=hv & 15, tq=tq))
            frame = dict(marker=marker, precision=p, height=h, width=w, components=comps)
        elif marker == 0xC4:
            i = 0
            while i < len(body):
                tc, th = body[i] >> 4, body[i] & 15
                counts = list(body[i + 1:i + 17])
                total = sum(counts)
                syms = list(body[i + 17:i + 17 + total])
                (dc_tables if tc == 0 else ac_tables)[th] = huffman(counts, syms)
                i += 17 + total
        elif marker == 0xDB:
            i = 0
            while i < len(body):
                pq, tq = body[i] >> 4, body[i] & 15
                if pq == 0:
                    quant[tq] = list(body[i + 1:i + 65])
                    i += 65
                else:
                    quant[tq] = list(struct.unpack(">64H", body[i + 1:i + 129]))
                    i += 129
        elif marker == 0xDD:
            (restart,) = struct.unpack(">H", body[:2])
        elif marker == 0xDA:
            n = body[0]
            sc = [dict(id=body[1 + 2 * i], dc=body[2 + 2 * i] >> 4, ac=body[2 + 2 * i] & 15) for i in range(n)]
            ss, se, a = body[1 + 2 * n:4 + 2 * n]
            scan = dict(components=sc, ss=ss, se=se, ah=a >> 4, al=a & 15)
            if frame is None or frame["marker"] not in (0xC0, 0xC1) or frame["precision"] != 8:
                scans.append(dict(at=at, run=run, refused=True))
                continue
            coefs, blocks, stats = decode_scan(data, run, frame, scan, dc_tables, ac_tables, restart)
            scans.append(dict(at=at, run=run, refused=False, coefs=coefs, blocks=blocks, stats=stats,
                              quant={m: quant[frame["components"][m]["tq"]] for m in range(len(frame["components"]))}))
    return frame, scans


def idct_planes(frame, scans):
    """Every component's samples at its own resolution, from all its scans."""
    import numpy as np
    n = np.arange(8)
    c = np.where(n == 0, np.sqrt(1 / 8), np.sqrt(2 / 8))
    basis = c[:, None] * np.cos((2 * n[None, :] + 1) * n[:, None] * np.pi / 16)  # [u, x]
    comps = frame["components"]
    hmax = max(c_["h"] for c_ in comps)
    vmax = max(c_["v"] for c_ in comps)
    planes = {}
    for s in scans:
        if s["refused"]:
            continue
        for (m, bx, by, natural) in s["blocks"]:
            q = s["quant"][m]
            qn = [0] * 64
            for j in range(64):
                qn[ZIGZAG[j]] = q[j]
            f = np.array(natural, dtype=float).reshape(8, 8) * np.array(qn, dtype=float).reshape(8, 8)
            px = basis.T @ f @ basis + 128
            cw = -(-frame["width"] * comps[m]["h"] // hmax)
            ch = -(-frame["height"] * comps[m]["v"] // vmax)
            plane = planes.setdefault(m, np.zeros(((-(-ch // 8) + 2) * 8 * vmax, (-(-cw // 8) + 2) * 8 * hmax)))
            plane[by * 8:by * 8 + 8, bx * 8:bx * 8 + 8] = px
    out = {}
    for m, plane in planes.items():
        cw = -(-frame["width"] * comps[m]["h"] // hmax)
        ch = -(-frame["height"] * comps[m]["v"] // vmax)
        out[m] = np.clip(np.round(plane[:ch, :cw]), 0, 255)
    return out


def pillow_check(path, frame, scans):
    import numpy as np
    from PIL import Image
    im = Image.open(path)
    comps = frame["components"]
    full = [m for m, c_ in enumerate(comps) if c_["h"] == max(x["h"] for x in comps) and c_["v"] == max(x["v"] for x in comps)]
    if im.mode == "RGB":
        im.draft("YCbCr", im.size)
    theirs = np.asarray(im, dtype=float)
    if theirs.ndim == 2:
        theirs = theirs[:, :, None]
    ours = idct_planes(frame, scans)
    worst = 0
    for m in full:
        a = ours[m]
        b = theirs[:, :, m]
        d = np.abs(a - b)
        # Pillow hands back an Adobe CMYK file with every sample inverted.
        if im.mode == "CMYK" and d.mean() > 64:
            d = np.abs(a - (255 - b))
        worst = max(worst, int(d.max()))
    return dict(channels=len(full), worst=worst)


def main():
    args = sys.argv[1:]
    pillow = "--pillow" in args
    args = [a for a in args if a != "--pillow"]
    offset = 0
    if "--offset" in args:
        i = args.index("--offset")
        offset = int(args[i + 1], 0)
        del args[i:i + 2]
    for path in args:
        data = open(path, "rb").read()
        frame, scans = read(data, offset)
        report = dict(file=path, width=frame["width"], height=frame["height"], scans=[])
        for s in scans:
            if s["refused"]:
                report["scans"].append(dict(at=s["at"], refused=True))
                continue
            report["scans"].append(dict(
                at=s["at"], run=list(s["run"]), blocks=len(s["blocks"]),
                crc=f"{zlib.crc32(s['coefs']):#010x}", **s["stats"]))
        if pillow and offset == 0 and all(not s["refused"] for s in scans):
            report["pillow"] = pillow_check(path, frame, scans)
        print(json.dumps(report))


if __name__ == "__main__":
    main()
