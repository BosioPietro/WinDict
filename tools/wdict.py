"""Writes WinDict's compact dictionary format, for the scripts that build one.

File layout (little endian):

    b"WDIC"  u32 version (4)
    u32 entry_block_count   u32 synset_block_count   u32 synsets_per_block
    u32 info_offset   u32 info_length
    entry_block_count  x (u32 first_key_offset, u32 data_offset, u32 data_length)
    synset_block_count x (u32 data_offset, u32 data_length)
    first keys of the entry blocks (NUL-terminated UTF-8)
    info: '\\n'-separated "name=value" lines (UTF-8): `language`, the code the
        app's `language` setting names it by; `name`, the source the card
        credits; `link`, its page for a word, with "{}" for the word
    raw-deflate blocks

Entry blocks hold consecutive entries, sorted by key in byte order and
separated by 0x1E. Keys are lowercase (str.lower), as the app looks them up.
An entry is '\\n'-separated lines: the key, then
    W<display form>   R<base form, for inflections>   S<spelling with accents, for words typed without>
    P<pos><space-separated synset ids>
where pos is n (noun), v (verb), a (adjective), r (adverb), p (phrase),
i (interjection), o (pronoun), e (preposition) or c (conjunction).
Synset blocks hold `synsets_per_block` synsets separated by 0x1E, each
'\\n'-separated: lemmas joined by '|', their antonyms (a group per lemma, in
the same order, joined by '|', each joined by '\\t'), the definition, then
examples.
"""

import struct
import unicodedata
import zlib

ENTRIES_PER_BLOCK = 64
SYNSETS_PER_BLOCK = 64


def new_entry():
    """An entry: "pos" holds (pos, total senses, [synset ids]) per part of speech."""
    return {"display": None, "redirects": [], "spellings": [], "pos": [], "forms": []}


def without_accents(text):
    return "".join(c for c in unicodedata.normalize("NFD", text) if not unicodedata.combining(c))


def add_spellings(entries):
    """Accented words, as they're often typed: "senor" for "señor"."""
    for key, entry in list(entries.items()):
        plain = without_accents(key)
        if plain != key and entry["pos"]:
            other = entries.setdefault(plain, new_entry())
            if not other["pos"] and key not in other["spellings"]:
                other["spellings"].append(key)


def deflate(text):
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    return c.compress(text.encode("utf-8")) + c.flush()


def clean(text):
    return text.replace("\x1e", " ").replace("\n", " ")


def write(out, entries, synsets, antonyms, info):
    """`synsets` are (lemmas, definition, examples); `antonyms` hold, per
    synset, a list of opposites per lemma; `info` is what the file says
    about itself."""

    def entry_text(key, e):
        lines = [key]
        if e["display"] and e["display"] != key:
            lines.append("W" + e["display"])
        lines += ["R" + r for r in e["redirects"]]
        lines += ["S" + s for s in e["spellings"]]
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

    header_len = 28 + 12 * len(entry_blocks) + 8 * len(synset_blocks)
    key_area = b"".join(b[0].encode("utf-8") + b"\0" for b in entry_blocks)
    info = "\n".join(f"{k}={v}" for k, v in info.items()).encode("utf-8")
    data_base = header_len + len(key_area) + len(info)
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
        f.write(b"WDIC" + struct.pack("<IIIIII", 4, len(entry_blocks), len(synset_blocks), SYNSETS_PER_BLOCK,
                                      header_len + len(key_area), len(info)))
        for t in entry_table:
            f.write(struct.pack("<III", *t))
        for t in synset_table:
            f.write(struct.pack("<II", *t))
        f.write(key_area)
        f.write(info)
        f.write(data)
    pairs = sum(len(a) for s in antonyms for a in s)
    print(f"{len(keys)} entries, {len(synsets)} synsets, {pairs} antonyms, {data_base + len(data)} bytes")
