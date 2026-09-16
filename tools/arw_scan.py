"""Walk every IFD of a TIFF-based camera raw and name each tag from the tables in
crates/core/src/formats/tiff.rs, printing the tags no table names.

A one-off from 2026-09-15 that the Rust test
`sony_arw_real_ifds_name_all_observed_tags` in tiff.rs now covers; kept as the
quick way to eyeball a new raw sample. Run from the repo root:

    python3 tools/arw_scan.py [path-to-raw]
"""
from pathlib import Path
import os, re, struct, sys

samples = Path(os.environ.get("QUBERO_SAMPLES", Path(__file__).resolve().parents[2] / "qubero-samples"))
file = Path(sys.argv[1]) if len(sys.argv) > 1 else samples / "cameraraw" / "sony-ilce-7s-14bit-compressed.arw"
data = file.read_bytes()
source = Path("crates/core/src/formats/tiff.rs").read_text()
tagsets = {}
for name, end in (("Tiff", "EXIF_TAG"), ("Exif", "GPS_TAG"), ("Gps", "FIELD_TYPE")):
    table = {"Tiff": "TAG", "Exif": "EXIF_TAG", "Gps": "GPS_TAG"}[name]
    start = source.index(f"const {table}:")
    stop = source.index(f"const {end}:", start)
    tagsets[name] = {int(n): label for n, label in re.findall(r'\((\d+), "([^"]+)"\)', source[start:stop])}

endian = "<" if data[:2] == b"II" else ">"
u16 = lambda n: struct.unpack_from(endian + "H", data, n)[0]
u32 = lambda n: struct.unpack_from(endian + "I", data, n)[0]
print("size", len(data), "endian", endian, "version", u16(2), "root", hex(u32(4)))
seen = set()
def scan(offset, space="Tiff", label="IFD", depth=0):
    if offset in seen or offset < 8 or offset + 2 > len(data) or depth > 8:
        return
    seen.add(offset)
    count = u16(offset)
    if count > 500 or offset + 2 + count * 12 + 4 > len(data):
        print("BAD IFD", hex(offset), count)
        return
    print("\n", label, hex(offset), "entries", count)
    subdirs = []
    for i in range(count):
        at = offset + 2 + i * 12
        tag, typ, qty, val = struct.unpack_from(endian + "HHII", data, at)
        name = tagsets[space].get(tag)
        print(f"  @{at:#x} tag={tag:<5} type={typ:<3} qty={qty:<6} val={val:#x} {name or 'UNKNOWN'}")
        if tag in (330, 34665, 34853) and typ in (4, 13):
            offsets = [val] if qty == 1 else [u32(val + j * 4) for j in range(min(qty, 30)) if val + j * 4 + 4 <= len(data)]
            to = "Exif" if tag == 34665 else "Gps" if tag == 34853 else "Tiff"
            subdirs += [(v, to, f"sub {tag}") for v in offsets]
    nextpos = offset + 2 + count * 12
    following = u32(nextpos)
    if following:
        subdirs.append((following, space, "next IFD"))
    for args in subdirs:
        scan(*args, depth=depth + 1)

scan(u32(4))
