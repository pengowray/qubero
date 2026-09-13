import os, sys, yaml
root = sys.argv[1]
rows = []
for d, _, fs in os.walk(root):
    if "_build" in d: continue
    for f in fs:
        if not f.endswith(".ksy"): continue
        p = os.path.join(d, f)
        with open(p, encoding="utf-8") as fh: data = yaml.safe_load(fh)
        m = data.get("meta", {})
        ext = m.get("file-extension", "")
        if isinstance(ext, list): ext = ",".join(map(str, ext))
        seq = data.get("seq") or []
        lead = ""
        if seq and isinstance(seq[0], dict) and "contents" in seq[0]:
            lead = "magic0"
        cat = os.path.relpath(d, root).replace("\\", "/")
        rows.append((cat, m.get("id"), str(m.get("title", ""))[:50], m.get("license", "?"), ext, os.path.getsize(p), "imports" if m.get("imports") else "", lead))
rows.sort()
for r in rows: print("\t".join(map(str, r)))
