#!/usr/bin/env python3
"""Converts a WordNet database (WNDB format) into WinDict's compact dictionary.

Usage: python tools/build_dict.py <wordnet dir> data/english.wdict

The WNDB files of Open English WordNet ship in NLTK's `english_wordnet`
package: https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/english_wordnet.zip

File layout (little endian):

    b"WDIC"  u32 version (3)
    u32 entry_block_count   u32 synset_block_count   u32 synsets_per_block
    entry_block_count  x (u32 first_key_offset, u32 data_offset, u32 data_length)
    synset_block_count x (u32 data_offset, u32 data_length)
    first keys of the entry blocks (NUL-terminated UTF-8)
    raw-deflate blocks

Entry blocks hold consecutive entries, sorted by key in byte order and
separated by 0x1E. An entry is '\\n'-separated lines: the key, then
    W<display form>   R<base form, for inflections>   P<pos (n v a r)><space-separated synset ids>
Synset blocks hold `synsets_per_block` synsets separated by 0x1E, each
'\\n'-separated: lemmas joined by '|', their antonyms (a group per lemma, in
the same order, joined by '|', each joined by '\\t'), the definition, then
examples.
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


def ends_in_consonant_y(word):
    return len(word) > 2 and word[-1] == "y" and word[-2] not in "aeiou"


def doubles(word):
    """Whether `word` doubles its last letter before a suffix ("hop" → "hopping")."""
    return (len(word) >= 3 and word[-1] not in "aeiouwxy" and word[-2] in "aeiou"
            and word[-3] not in "aeiou")


def s_form(word):
    """A noun's plural or a verb's third person: "box" → "boxes", "city" → "cities"."""
    if word.endswith(("s", "x", "z", "ch", "sh")):
        return word + "es"
    if ends_in_consonant_y(word):
        return word[:-1] + "ies"
    return word + "s"


def noun_forms(word):
    forms = [s_form(word)]
    if word.endswith("man"):
        forms.append(word[:-3] + "men")
    return forms


def verb_forms(word):
    if word.endswith("e"):
        past = word + "d"
    elif ends_in_consonant_y(word):
        past = word[:-1] + "ied"
    else:
        past = word + "ed"
    if word.endswith("ie"):
        gerund = word[:-2] + "ying"
    elif word.endswith("e") and not word.endswith(("ee", "ye", "oe")):
        gerund = word[:-1] + "ing"
    else:
        gerund = word + "ing"
    return [s_form(word), past, gerund]


def adjective_forms(word):
    if word.endswith("e"):
        return [word + "r", word + "st"]
    if ends_in_consonant_y(word):
        return [word[:-1] + "ier", word[:-1] + "iest"]
    return [word + "er", word + "est"]


def inflections(key, pos):
    """The regular inflections of `key` as part of speech `pos`. Irregular
    ones ("ran", "geese") come from WordNet's exception lists instead. Phrases
    inflect their head: a noun's last word ("ice creams"), a verb's first
    ("looking up"). Some forms don't exist ("runned"), but nobody types them."""
    words = key.split(" ")
    if pos == "n":
        i, rules = len(words) - 1, noun_forms
    elif pos == "v":
        i, rules = 0, verb_forms
    elif pos == "a" and len(words) == 1:
        i, rules = 0, adjective_forms
    else:
        return []
    if not (words[i].isalpha() and len(words[i]) >= 2):
        return []
    return [" ".join(words[:i] + [form] + words[i + 1:]) for form in rules(words[i])]


def deflate(text):
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    return c.compress(text.encode("utf-8")) + c.flush()


def clean(text):
    return text.replace("\x1e", " ").replace("\n", " ")


def main(root, out):
    root = Path(root)
    synsets = []  # (lemmas, definition, examples)
    ids = {}  # (pos, offset) -> id
    pointers = []  # antonyms: (synset id, lemma index, (pos, offset), their lemma index)
    for pos, name in POS_FILES.items():
        for line in (root / f"data.{name}").open(encoding="utf-8"):
            if not line[:1].isdigit():
                continue
            head, _, gloss = line.partition(" | ")
            fields = head.split()
            count = int(fields[3], 16)
            lemmas = [clean_lemma(fields[4 + 2 * i]) for i in range(count)]
            rest = fields[4 + 2 * count:]
            for i in range(int(rest[0])):
                symbol, offset, target_pos, source_target = rest[1 + 4 * i:5 + 4 * i]
                if symbol == "!":
                    # Satellite adjectives (s) are in data.adj too.
                    target_pos = "a" if target_pos == "s" else target_pos
                    pointers.append((len(synsets), int(source_target[:2], 16) - 1,
                                     (target_pos, offset), int(source_target[2:], 16) - 1))
            ids[(pos, fields[0])] = len(synsets)
            synsets.append((lemmas, *parse_gloss(gloss)))

    antonyms = [[[] for _ in s[0]] for s in synsets]  # per synset, per lemma
    for synset, lemma, target, target_lemma in pointers:
        word = synsets[ids[target]][0][target_lemma]
        if word not in antonyms[synset][lemma]:
            antonyms[synset][lemma].append(word)

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

    # Regular inflections, for the words that aren't words of their own: a
    # form of several words points at the one with the most senses first.
    bases = {}  # form -> [(senses, base)]
    for key, entry in list(entries.items()):
        if entry["display"] != key:
            continue  # only capitalised ("Paris")
        senses = sum(total for _, total, _ in entry["pos"])
        for pos, _, _ in entry["pos"]:
            for form in inflections(key, pos):
                if form != key and (senses, key) not in bases.setdefault(form, []):
                    bases[form].append((senses, key))
    for form, found in bases.items():
        entry = entries.setdefault(form, {"display": None, "redirects": [], "pos": [], "forms": []})
        if entry["pos"]:
            continue
        # "hoping" is from "hope", not "hop", which doubles its last letter.
        keys = {b for _, b in found}
        found.sort(key=lambda b: (doubles(b[1]) and b[1] + "e" in keys, -b[0]))
        entry["redirects"] += [b for _, b in found if b not in entry["redirects"]]

    def entry_text(key, e):
        lines = [key]
        if e["display"] and e["display"] != key:
            lines.append("W" + e["display"])
        lines += ["R" + r for r in e["redirects"]]
        for pos, _, sense_ids in sorted(e["pos"], key=lambda p: -p[1]):  # most senses first
            lines.append("P" + pos + " ".join(map(str, sense_ids)))
        return "\n".join(clean(l) for l in lines)

    def synset_text(s, opposites):
        lemmas, definition, examples = s
        opposites = "|".join("\t".join(a) for a in opposites)
        return "\n".join(clean(l) for l in ["|".join(lemmas), opposites, definition, *examples[:2]])

    keys = sorted(entries, key=lambda k: k.encode("utf-8"))
    entry_blocks = [keys[i:i + ENTRIES_PER_BLOCK] for i in range(0, len(keys), ENTRIES_PER_BLOCK)]
    synset_blocks = [range(i, min(i + SYNSETS_PER_BLOCK, len(synsets)))
                     for i in range(0, len(synsets), SYNSETS_PER_BLOCK)]

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
        packed = deflate("\x1e".join(synset_text(synsets[i], antonyms[i]) for i in block))
        synset_table.append((data_base + len(data), len(packed)))
        data += packed

    with open(out, "wb") as f:
        f.write(b"WDIC" + struct.pack("<IIII", 3,len(entry_blocks), len(synset_blocks), SYNSETS_PER_BLOCK))
        for t in entry_table:
            f.write(struct.pack("<III", *t))
        for t in synset_table:
            f.write(struct.pack("<II", *t))
        f.write(key_area)
        f.write(data)
    pairs = sum(len(a) for s in antonyms for a in s)
    print(f"{len(keys)} entries, {len(synsets)} synsets, {pairs} antonyms, {data_base + len(data)} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
