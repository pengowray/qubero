"""Count what appears inside .ksy expressions across the Kaitai formats corpus.

Usage: python ksy_stats.py <root> [<root2> ...]
Prints a table: construct, files using it, total occurrences.
"""
import re, sys, os, collections
import yaml

EXPR_KEYS = {"size", "repeat-expr", "repeat-until", "if", "value", "pos", "io",
             "switch-on", "valid", "to-string", "process", "encoding", "terminator", "pad-right"}

def walk(node, path, out):
    """Collect (key, expression-string, path) for every expression-valued key."""
    if isinstance(node, dict):
        for k, v in node.items():
            p = path + [str(k)]
            if k in EXPR_KEYS and isinstance(v, (str, int, bool, float)):
                out.append((k, str(v), p))
            elif k == "valid" and isinstance(v, dict):
                for vk, vv in v.items():
                    out.append(("valid." + str(vk), str(vv), p))
            elif k == "type" and isinstance(v, dict) and "switch-on" in v:
                out.append(("switch-on", str(v["switch-on"]), p))
                for ck in v.get("cases", {}):
                    out.append(("case-key", str(ck), p))
            elif k == "type" and isinstance(v, str) and "(" in v:
                out.append(("type-args", v, p))
            elif k == "endian" and isinstance(v, dict):
                out.append(("switch-endian", str(v.get("switch-on")), p))
            walk(v, p, out)
    elif isinstance(node, list):
        for i, v in enumerate(node):
            walk(v, path + [str(i)], out)

PATTERNS = [
    ("modulo %", r"(?<![%\w])%(?!\w*})"),
    ("ternary ?:", r"\?[^?]*:"),
    ("_io.pos", r"_io\.pos\b"),
    ("_io.size", r"_io\.size\b"),
    ("_io.eof", r"_io\.eof\b"),
    ("_root.", r"\b_root\."),
    ("_parent.", r"\b_parent\."),
    ("_parent._parent", r"_parent\._parent"),
    ("_index", r"\b_index\b"),
    ("_ (iterator)", r"(?<![\w.])_(?![\w])"),
    ("_on", r"\b_on\b"),
    (".to_s", r"\.to_s\b"),
    (".to_i", r"\.to_i\b"),
    (".length", r"\.length\b"),
    (".size (arr/bytes)", r"(?<!_io)\.size\b"),
    (".substring", r"\.substring\b"),
    (".first/.last", r"\.(first|last)\b"),
    (".min/.max", r"\.(min|max)\b"),
    (".reverse", r"\.reverse\b"),
    (".as<>", r"\.as<"),
    ("sizeof<>", r"\bsizeof<"),
    ("bitsizeof<>", r"\bbitsizeof<"),
    ("string literal", r"(\"[^\"]*\"|'[^']*')"),
    ("string ==", r"(\"[^\"]*\"|'[^']*')\s*[!=]=|[!=]=\s*(\"[^\"]*\"|'[^']*')"),
    ("float literal", r"\b\d+\.\d+\b"),
    ("byte array literal", r"\[\s*0x"),
    ("enum::label", r"\w+::\w+"),
    ("and/or/not", r"\b(and|or|not)\b"),
    ("bit ops & | ^ ~", r"[&|^~]"),
    ("shift << >>", r"<<|>>"),
    ("comparison", r"[<>]=?|[!=]="),
    ("arithmetic + - * /", r"[-+*/](?!\w*>)"),
    ("subscript []", r"\w\[[^\]]+\]"),
    ("method call ()", r"\.\w+\("),
    ("f-string", r"f\""),
]

def main(roots):
    files = []
    for root in roots:
        for d, _, fs in os.walk(root):
            if "_build" in d: continue
            files += [os.path.join(d, f) for f in fs if f.endswith(".ksy")]
    per_key = collections.Counter(); per_key_files = collections.Counter()
    pat_files = collections.Counter(); pat_total = collections.Counter()
    io_variants = collections.Counter(); until_shapes = collections.Counter()
    type_arg_shapes = collections.Counter(); pos_anchor = collections.Counter()
    repeat_until_examples = []
    doc_fields = 0; total_fields = 0; webide = 0; orig = 0
    if_examples = []
    for f in files:
        try:
            with open(f, encoding="utf-8") as fh:
                data = yaml.safe_load(fh)
        except Exception as e:
            print("YAML FAIL", f, e); continue
        exprs = []
        walk(data, [], exprs)
        seen_keys = set(); seen_pats = set()
        for k, e, p in exprs:
            per_key[k] += 1; seen_keys.add(k)
            for name, rx in PATTERNS:
                n = len(re.findall(rx, e))
                if n:
                    pat_total[name] += n; seen_pats.add(name)
            if k == "io":
                io_variants[e] += 1
            if k == "repeat-until":
                shape = re.sub(r"\b\d+\b", "N", e)
                shape = re.sub(r"0x[0-9a-fA-F]+", "N", shape)
                until_shapes["_.field == lit" if re.fullmatch(r"_\.\w+ == N", shape) else
                             "_ == lit" if re.fullmatch(r"_ == N", shape) else
                             "_io.eof" if "_io.eof" in shape else "other"] += 1
                if len(repeat_until_examples) < 40: repeat_until_examples.append(e)
            if k == "type-args":
                args = e[e.index("(")+1:-1]
                type_arg_shapes["all-literal" if re.fullmatch(r"[\w\s,'\"]*", args) and not re.search(r"[a-z_]\w*(?![\w'\"])", args.replace("true","").replace("false","")) else "has-ref"] += 1
            if k == "pos":
                pos_anchor["mentions _io/_root" if "_root" in e or "_io" in e else "plain"] += 1
            if k == "if" and len(if_examples) < 30: if_examples.append(e)
        for k in seen_keys: per_key_files[k] += 1
        for n in seen_pats: pat_files[n] += 1
        # doc coverage on seq fields
        def seqs(node):
            if isinstance(node, dict):
                for k, v in node.items():
                    if k == "seq" and isinstance(v, list):
                        for a in v:
                            if isinstance(a, dict): yield a
                    else: yield from seqs(v)
            elif isinstance(node, list):
                for v in node: yield from seqs(v)
        for a in seqs(data):
            total_fields += 1
            if "doc" in a: doc_fields += 1
            if "-webide-representation" in a: webide += 1
            if "-orig-id" in a: orig += 1
    print(f"files: {len(files)}   seq fields: {total_fields}  with doc: {doc_fields}  with -webide-representation: {webide}  with -orig-id: {orig}\n")
    print("| expression key | files | occurrences |\n|---|---|---|")
    for k, n in per_key.most_common(): print(f"| {k} | {per_key_files[k]} | {n} |")
    print("\n| construct inside expressions | files | occurrences |\n|---|---|---|")
    for name, _ in PATTERNS: print(f"| {name} | {pat_files[name]} | {pat_total[name]} |")
    print("\nio: variants:", dict(io_variants.most_common()))
    print("repeat-until shapes:", dict(until_shapes))
    print("repeat-until examples:", repeat_until_examples)
    print("type args:", dict(type_arg_shapes))
    print("pos anchors:", dict(pos_anchor))
    print("if examples:", if_examples)

main(sys.argv[1:])
