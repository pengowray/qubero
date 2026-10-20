"""Write what the asdf library reads from ASDF files, for the template to be
checked against.

For every numeric array the tree points at a block with, this writes the block
number, the shape, the datatype, and the values at some places: the first and
last elements, the middle, a few chosen at random but seeded, and the two
either side of every 4 MiB of bytes, which is where asdf's lz4 starts a new
chunk. The template reads a compressed block by joining its chunks, so those
are the elements most likely to come out wrong.

    python -I tools/asdf_reference.py OUT.tsv FILE.asdf [FILE.asdf ...]

One line a value: the file, the array's path in the tree, its shape, its
dtype, the element's index and the value. A float is written as `repr` gives
it, so a 16-bit float survives exactly, and NaN and the infinities as `nan`,
`inf` and `-inf`.
"""

import os
import sys

import asdf
import numpy as np

CHUNK = 4 << 20


def text(v):
    if isinstance(v, (np.floating, float)):
        return repr(float(v))
    return str(int(v))


def places(a):
    n = a.size
    if n == 0:
        return []
    flat = {0, n - 1, n // 2}
    rng = np.random.default_rng(0)
    flat.update(int(i) for i in rng.integers(0, n, size=min(n, 20)))
    for k in range(1, a.nbytes // CHUNK + 1):
        edge = k * CHUNK // a.itemsize
        flat.update(i for i in (edge - 1, edge) if 0 <= i < n)
    return sorted(flat)


def arrays(node, path, out):
    if isinstance(node, dict):
        for k, v in node.items():
            arrays(v, f"{path}.{k}" if path else str(k), out)
    elif isinstance(node, (list, tuple)):
        for i, v in enumerate(node):
            arrays(v, f"{path}[{i}]", out)
    elif isinstance(node, np.ndarray):
        out.append((path, node))


def read(path):
    out = {}
    with asdf.open(path, lazy_load=False, memmap=False) as af:
        # Run without the roman_datamodels and gwcs packages, so their tags stay
        # the dicts and lists the file wrote and the paths are the tree's own.
        found = []
        arrays(af.tree, "", found)
        for name, a in found:
            # Only what a block holds: an array written inline is part of the
            # tree's text, and an external one is in another file.
            if a.dtype.kind not in "iufb" or a.dtype.names or af.get_array_storage(a) not in ("internal", "streamed"):
                continue
            entry = {"shape": list(a.shape), "dtype": a.dtype.str, "values": []}
            flat = np.ascontiguousarray(a).reshape(-1)
            for i in places(a):
                at = [int(x) for x in np.unravel_index(i, a.shape)]
                entry["values"].append([at, text(flat[i])])
            out[name] = entry
    return out


def main():
    target, files = sys.argv[1], sys.argv[2:]
    result = {os.path.basename(f): read(f) for f in files}
    with open(target, "w") as fd:
        fd.write("# file\tarray\tshape\tdtype\tindex\tvalue\n")
        for f, found in result.items():
            for name, a in found.items():
                shape = ",".join(map(str, a["shape"]))
                for at, value in a["values"]:
                    fd.write(f"{f}\t{name}\t{shape}\t{a['dtype']}\t{','.join(map(str, at))}\t{value}\n")
    for f, arrays_ in result.items():
        print(f"{f}: {len(arrays_)} arrays, {sum(len(a['values']) for a in arrays_.values())} values")


if __name__ == "__main__":
    main()
