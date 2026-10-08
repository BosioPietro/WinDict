#!/usr/bin/env python3
"""Converts a WordNet database (WNDB format) into WinDict's compact dictionary.

Usage: python tools/build_dict.py <wordnet dir> data/english.wdict

The WNDB files of Open English WordNet ship in NLTK's `english_wordnet`
package: https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/english_wordnet.zip

File layout (little endian):

    b"WDIC"  u32 version (2)
    u32 entry_block_count   u32 synset_block_count   u32 synsets_per_block
    entry_block_count  x (u32 first_key_offset, u32 data_offset, u32 data_length)
    synset_block_count x (u32 data_offset, u32 data_length)
    first keys of the entry blocks (NUL-terminated UTF-8)
    raw-deflate blocks

Entry blocks hold consecutive entries, sorted by key in byte order and
separated by 0x1E. An entry is '\\n'-separated lines: the key, then
    W<display form>   R<base form, for inflections>   P<pos (n v a r)><space-separated synset ids>
Synset blocks hold `synsets_per_block` synsets separated by 0x1E, each
'\\n'-separated: lemmas joined by '|', the definition, then examples.
"""

import re
import struct
import sys
import zlib
from pathlib import Path

POS_FILES = {"n": "noun", "v": "verb", "a": "adj", "r": "adv"}
MAX_SENSES = 8
ENTRIES_PER_BLOCK = 64
SYNSETS_PER_BLOCK = 64


def clean_lemma(word):
    return re.sub(r"\((a|p|ip)\)$", "", word).replace("_", " ")


def parse_gloss(gloss):
    gloss = gloss.strip()
    examples = [e.strip() for e in re.findall(r'"([^"]+)"', gloss) if e.strip()]
    definition = gloss.split('"')[0].strip().rstrip(";").strip() if '"' in gloss else gloss
    return definition or gloss.strip(), examples


def deflate(text):
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    return c.compress(text.encode("utf-8")) + c.flush()


def clean(text):
    return text.replace("\x1e", " ").replace("\n", " ")


def main(root, out):
    root = Path(root)
    synsets = []  # (lemmas, definition, examples)
    ids = {}  # (pos, offset) -> id
    for pos, name in POS_FILES.items():
        for line in (root / f"data.{name}").open(encoding="utf-8"):
            if not line[:1].isdigit():
                continue
            head, _, gloss = line.partition(" | ")
            fields = head.split()
            count = int(fields[3], 16)
            lemmas = [clean_lemma(fields[4 + 2 * i]) for i in range(count)]
            ids[(pos, fields[0])] = len(synsets)
            synsets.append((lemmas, *parse_gloss(gloss)))

    entries = {}  # key -> {"display", "redirects", "pos": [(pos, total senses, [ids])]}
    for pos, name in POS_FILES.items():
        for line in (root / f"index.{name}").open(encoding="utf-8"):
            if line.startswith(" ") or not line.strip():
                continue
            fields = line.split()
            key = fields[0].replace("_", " ")
            offsets = fields[6 + int(fields[3]):]
            sense_ids = [ids[(pos, o)] for o in offsets[:MAX_SENSES]]
            entry = entries.setdefault(key, {"display": None, "redirects": [], "pos": [], "forms": []})
            entry["forms"] += [l for i in sense_ids for l in synsets[i][0] if l.lower() == key]
            entry["pos"].append((pos, len(offsets), sense_ids))

    # Keep the lowercase form unless the word only appears capitalised ("Paris").
    for key, entry in entries.items():
        entry["display"] = key if key in entry["forms"] or not entry["forms"] else entry["forms"][0]

    for name in POS_FILES.values():
        for line in (root / f"{name}.exc").open(encoding="utf-8"):
            words = [w.replace("_", " ") for w in line.split()]
            form, bases = words[0], [b for b in words[1:] if b in entries and b != words[0]]
            if not bases:
                continue
            entry = entries.setdefault(form, {"display": None, "redirects": [], "pos": [], "forms": []})
            entry["redirects"] += [b for b in bases if b not in entry["redirects"]]

    def entry_text(key, e):
        lines = [key]
        if e["display"] and e["display"] != key:
            lines.append("W" + e["display"])
        lines += ["R" + r for r in e["redirects"]]
        for pos, _, sense_ids in sorted(e["pos"], key=lambda p: -p[1]):  # most senses first
            lines.append("P" + pos + " ".join(map(str, sense_ids)))
        return "\n".join(clean(l) for l in lines)

    def synset_text(s):
        lemmas, definition, examples = s
        return "\n".join(clean(l) for l in ["|".join(lemmas), definition, *examples[:2]])

    keys = sorted(entries, key=lambda k: k.encode("utf-8"))
    entry_blocks = [keys[i:i + ENTRIES_PER_BLOCK] for i in range(0, len(keys), ENTRIES_PER_BLOCK)]
    synset_blocks = [synsets[i:i + SYNSETS_PER_BLOCK] for i in range(0, len(synsets), SYNSETS_PER_BLOCK)]

    header_len = 20 + 12 * len(entry_blocks) + 8 * len(synset_blocks)
    key_area = b"".join(b[0].encode("utf-8") + b"\0" for b in entry_blocks)
    data_base = header_len + len(key_area)
    data = bytearray()
    entry_table, synset_table = [], []
    key_off = header_len
    for block in entry_blocks:
        packed = deflate("\x1e".join(entry_text(k, entries[k]) for k in block))
        entry_table.append((key_off, data_base + len(data), len(packed)))
        key_off += len(block[0].encode("utf-8")) + 1
        data += packed
    for block in synset_blocks:
        packed = deflate("\x1e".join(synset_text(s) for s in block))
        synset_table.append((data_base + len(data), len(packed)))
        data += packed

    with open(out, "wb") as f:
        f.write(b"WDIC" + struct.pack("<IIII", 2, len(entry_blocks), len(synset_blocks), SYNSETS_PER_BLOCK))
        for t in entry_table:
            f.write(struct.pack("<III", *t))
        for t in synset_table:
            f.write(struct.pack("<II", *t))
        f.write(key_area)
        f.write(data)
    print(f"{len(keys)} entries, {len(synsets)} synsets, {data_base + len(data)} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
