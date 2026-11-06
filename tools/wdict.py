"""Writes WinDict's compact dictionary format, for the scripts that build one.

File layout (little endian):

    b"WDIC"  u32 version (5)
    u32 entry_block_count   u32 synset_block_count   u32 synsets_per_block
    u32 info_offset   u32 info_length
    u32 reverse_block_count
    entry_block_count   x (u32 first_key_offset, u32 data_offset, u32 data_length)
    synset_block_count  x (u32 data_offset, u32 data_length)
    reverse_block_count x (u32 first_key_offset, u32 data_offset, u32 data_length)
    first keys of the entry blocks (NUL-terminated UTF-8)
    first keys of the reverse blocks (NUL-terminated UTF-8)
    info: '\\n'-separated "name=value" lines (UTF-8): `language`, the code the
        app's `language` setting names it by; `name`, the source the card
        credits; `link`, its page for a word, with "{}" for the word
    raw-deflate blocks

Version 4 files are the same without the reverse blocks, their count and
their first keys: the header is 28 bytes rather than 32.

Entry blocks hold consecutive entries, sorted by key in byte order and
separated by 0x1E. Keys are lowercase (str.lower), as the app looks them up.
An entry is '\\n'-separated lines: the key, then
    W<display form>   R<base form, for inflections>   S<spelling with accents, for words typed without>
    P<pos><space-separated synset ids>
    T<language code> <translation>|<translation>   (the word in that language)
where pos is n (noun), v (verb), a (adjective), r (adverb), p (phrase),
i (interjection), o (pronoun), e (preposition) or c (conjunction).
Synset blocks hold `synsets_per_block` synsets separated by 0x1E, each
'\\n'-separated: lemmas joined by '|', their antonyms (a group per lemma, in
the same order, joined by '|', each joined by '\\t'), the definition, then
examples.
Reverse blocks are the translations the other way, for translating into the
dictionary's language: records like the entries', sorted by key, each the
key (the language code, a tab, the foreign word in lowercase) and then one
line per word of the dictionary's language it translates, best first.
"""

import hashlib
import struct
import unicodedata
import zlib

ENTRIES_PER_BLOCK = 64
SYNSETS_PER_BLOCK = 64
VERSION = 5


def new_entry():
    """An entry: "pos" holds (pos, total senses, [synset ids]) per part of
    speech; "translations", the word in other languages, by their code."""
    return {"display": None, "redirects": [], "spellings": [], "pos": [], "forms": [],
            "translations": {}}


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


def write(out, entries, synsets, antonyms, info, reverse=None):
    """`synsets` are (lemmas, definition, examples); `antonyms` hold, per
    synset, a list of opposites per lemma; `info` is what the file says
    about itself; `reverse` maps (language code, foreign word in lowercase)
    to the words of the dictionary's language it translates, best first."""
    reverse = reverse or {}

    def entry_text(key, e):
        lines = [key]
        if e["display"] and e["display"] != key:
            lines.append("W" + e["display"])
        lines += ["R" + r for r in e["redirects"]]
        lines += ["S" + s for s in e["spellings"]]
        for pos, _, sense_ids in sorted(e["pos"], key=lambda p: -p[1]):  # most senses first
            lines.append("P" + pos + " ".join(map(str, sense_ids)))
        for code, words in sorted(e.get("translations", {}).items()):
            words = [w.replace("|", " ") for w in words if w]
            if words:
                lines.append("T" + code + " " + "|".join(words))
        return "\n".join(clean(l) for l in lines)

    def synset_text(s, opposites):
        lemmas, definition, examples = s
        opposites = "|".join("\t".join(a) for a in opposites)
        return "\n".join(clean(l) for l in ["|".join(lemmas), opposites, definition, *examples[:2]])

    def reverse_text(key, words):
        code, word = key
        return "\n".join(clean(l) for l in [code + "\t" + word, *words])

    def blocks_of(keys):
        return [keys[i:i + ENTRIES_PER_BLOCK] for i in range(0, len(keys), ENTRIES_PER_BLOCK)]

    def key_bytes(key):
        if isinstance(key, tuple):
            key = key[0] + "\t" + key[1]
        return key.encode("utf-8")

    keys = sorted(entries, key=key_bytes)
    entry_blocks = blocks_of(keys)
    synset_blocks = [range(i, min(i + SYNSETS_PER_BLOCK, len(synsets)))
                     for i in range(0, len(synsets), SYNSETS_PER_BLOCK)]
    reverse_keys = sorted(reverse, key=key_bytes)
    reverse_blocks = blocks_of(reverse_keys)

    header_len = 32 + 12 * len(entry_blocks) + 8 * len(synset_blocks) + 12 * len(reverse_blocks)
    key_area = b"".join(key_bytes(b[0]) + b"\0" for b in entry_blocks)
    reverse_key_area = b"".join(key_bytes(b[0]) + b"\0" for b in reverse_blocks)
    info_text = "\n".join(f"{k}={v}" for k, v in info.items()).encode("utf-8")
    data_base = header_len + len(key_area) + len(reverse_key_area) + len(info_text)
    data = bytearray()
    entry_table, synset_table, reverse_table = [], [], []
    key_off = header_len
    for block in entry_blocks:
        packed = deflate("\x1e".join(entry_text(k, entries[k]) for k in block))
        entry_table.append((key_off, data_base + len(data), len(packed)))
        key_off += len(key_bytes(block[0])) + 1
        data += packed
    for block in synset_blocks:
        packed = deflate("\x1e".join(synset_text(synsets[i], antonyms[i]) for i in block))
        synset_table.append((data_base + len(data), len(packed)))
        data += packed
    for block in reverse_blocks:
        packed = deflate("\x1e".join(reverse_text(k, reverse[k]) for k in block))
        reverse_table.append((key_off, data_base + len(data), len(packed)))
        key_off += len(key_bytes(block[0])) + 1
        data += packed

    with open(out, "wb") as f:
        f.write(b"WDIC" + struct.pack("<IIIIIII", VERSION, len(entry_blocks), len(synset_blocks),
                                      SYNSETS_PER_BLOCK, header_len + len(key_area) + len(reverse_key_area),
                                      len(info_text), len(reverse_blocks)))
        for t in entry_table:
            f.write(struct.pack("<III", *t))
        for t in synset_table:
            f.write(struct.pack("<II", *t))
        for t in reverse_table:
            f.write(struct.pack("<III", *t))
        f.write(key_area)
        f.write(reverse_key_area)
        f.write(info_text)
        f.write(data)
    pairs = sum(len(a) for s in antonyms for a in s)
    translated = sum(len(w) for e in entries.values() for w in e.get("translations", {}).values())
    print(f"{len(keys)} entries, {len(synsets)} synsets, {pairs} antonyms, {translated} translations, "
          f"{len(reverse_keys)} reverse translations, {data_base + len(data)} bytes")
    with open(out, "rb") as f:
        sha256 = hashlib.sha256(f.read()).hexdigest()
    # As src/dicts.rs lists it, for a dictionary that's downloaded.
    print(f'file: "{info["language"]}-{sha256[:12]}.wdict", size: {data_base + len(data)}, sha256: "{sha256}"')
