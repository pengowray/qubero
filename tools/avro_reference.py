"""Write what fastavro reads from Avro files, for the template to be checked
against.

Every value of every record is written, each at its path in the record:
`diaSource.ra`, `points[2].x`, `labels["key0"]`, and `counts#` for how many
items an array or a map holds, so that an empty one is checked too. A union is
not a step in the path; its value is.

    python -I tools/avro_reference.py OUT.tsv FILE.avro [FILE.avro ...]

An object container file carries its schema. A file that starts with a zero
byte is one record in the Confluent wire format, and is read with the bundled
Rubin alert schema its ID names, from crates/core/formats-avro/alert_packet,
loaded the way lsst.alert.packet's Schema.from_file loads it.

One line a value: the file, the record's index, the path, and the value. A
long with a timestamp logical type is written as the number it is, not as a
date. Text is written as JSON, so that a tab or a newline in it survives. A
float is written as `repr` gives it, NaN and the infinities as `nan`, `inf`
and `-inf`. Bytes are written as `bytes:` and their length, then their CRC-32
in hex. An enum is its symbol as JSON; null is `null`.
"""

import io
import json
import math
import os
import struct
import sys
import zlib

import fastavro
import fastavro._read
import fastavro.read

# Every logical type read as the type it is written as: a timestamp as its
# long, a decimal as its bytes. The template reads them that way too.
fastavro.read.LOGICAL_READERS.clear()
fastavro._read.LOGICAL_READERS.clear()

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
ALERTS = os.path.join(ROOT, "crates", "core", "formats-avro", "alert_packet")


def plain(schema):
    """The schema with every logical type taken off, so a timestamp reads as
    the long it is written as."""
    if isinstance(schema, dict):
        return {k: plain(v) for k, v in schema.items() if k != "logicalType" and not k.startswith("__")}
    if isinstance(schema, list):
        return [plain(v) for v in schema]
    return schema


def text(v):
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        if math.isnan(v):
            return "nan"
        if math.isinf(v):
            return "inf" if v > 0 else "-inf"
        return repr(v)
    if isinstance(v, (bytes, bytearray)):
        return f"bytes:{len(v)}:{zlib.crc32(v):08x}"
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    raise TypeError(f"no text for {type(v)}")


def walk(v, schema, path, out, named):
    """Every value under `v`, which is of type `schema`. Records and maps are
    both dicts in fastavro, so the schema says which this is."""
    if isinstance(schema, str) and schema in named:
        schema = named[schema]
    if isinstance(schema, list):
        if v is None:
            out.append((path, "null"))
            return
        # The branch fastavro picked is not said; any branch the value fits
        # walks it the same way, except a record against a map.
        for b in schema:
            b = named.get(b, b) if isinstance(b, str) else b
            kind = b.get("type") if isinstance(b, dict) else b
            if kind == "null":
                continue
            if isinstance(v, dict) and kind not in ("record", "error", "map"):
                continue
            if isinstance(v, list) and kind != "array":
                continue
            if isinstance(v, dict) and kind in ("record", "error") and set(v) != {f["name"] for f in b["fields"]}:
                continue
            return walk(v, b, path, out, named)
        return walk(v, schema[-1], path, out, named)
    kind = schema.get("type") if isinstance(schema, dict) else schema
    if isinstance(kind, (dict, list)):
        return walk(v, kind, path, out, named)
    if kind in ("record", "error"):
        named[schema["name"]] = schema
        for f in schema["fields"]:
            walk(v[f["name"]], f["type"], f"{path}.{f['name']}" if path else f["name"], out, named)
    elif kind == "array":
        out.append((f"{path}#", str(len(v))))
        for i, x in enumerate(v):
            walk(x, schema["items"], f"{path}[{i}]", out, named)
    elif kind == "map":
        out.append((f"{path}#", str(len(v))))
        for k, x in v.items():
            walk(x, schema["values"], f"{path}[{json.dumps(k, ensure_ascii=False)}]", out, named)
    else:
        if kind in ("enum", "fixed"):
            named[schema["name"]] = schema
        out.append((path, text(v)))


def names(schema, out):
    """Every named type in a schema, by its full name and its short one."""
    if isinstance(schema, dict):
        if schema.get("type") in ("record", "error", "enum", "fixed"):
            out[schema["name"]] = schema
            out[schema["name"].rsplit(".", 1)[-1]] = schema
        for v in schema.values():
            names(v, out)
    elif isinstance(schema, list):
        for v in schema:
            names(v, out)
    return out


def read(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:1] == b"\0":
        (sid,) = struct.unpack(">I", data[1:5])
        major, minor = divmod(sid, 100)
        loaded = fastavro.schema.load_schema(os.path.join(ALERTS, str(major), str(minor), f"lsst.v{major}_{minor}.alert.avsc"))
        raw = plain(loaded)
        records = [fastavro.schemaless_reader(io.BytesIO(data[5:]), fastavro.parse_schema(raw))]
    else:
        with open(path, "rb") as f:
            raw = plain(json.loads(fastavro.reader(f).metadata["avro.schema"]))
        with open(path, "rb") as f:
            records = list(fastavro.reader(f))
    out = []
    for i, r in enumerate(records):
        found = []
        walk(r, raw, "", found, names(raw, {}))
        out.extend((i, p, v) for p, v in found)
    return len(records), out


def main():
    target, files = sys.argv[1], sys.argv[2:]
    with open(target, "w", encoding="utf-8", newline="\n") as fd:
        fd.write("# file\trecord\tpath\tvalue\n")
        for f in files:
            count, found = read(f)
            name = os.path.basename(f)
            fd.write(f"{name}\t-\t#records\t{count}\n")
            for i, p, v in found:
                fd.write(f"{name}\t{i}\t{p}\t{v}\n")
            print(f"{name}: {count} records, {len(found)} values")


if __name__ == "__main__":
    main()
