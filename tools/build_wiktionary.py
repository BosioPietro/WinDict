#!/usr/bin/env python3
"""Converts a Wiktionary extract by wiktextract (kaikki.org) into WinDict's
compact dictionary, for the words of one language.

Usage: python tools/build_wiktionary.py <extract.jsonl[.gz]> <language> <out.wdict>

For a dictionary in the language it defines, use that language's own
Wiktionary, e.g. the Italian one's for Italian:
https://kaikki.org/itwiktionary/raw-wiktextract-data.jsonl.gz

Each word's translations into the languages WinDict translates between go
in too, and the other way round (the words of this language a foreign word
translates to), so that single words can be translated offline. The other
way round, the words are put best first by how common they are, from the
lists in data/frequency.bin (tools/build_frequency.py), when that's there.

The file format is described in `wdict.py`, which writes it.
"""

import gzip
import json
import re
import struct
import sys
import zlib
from pathlib import Path

from wdict import add_spellings, new_entry, write

# Each Wiktionary's name, as the card credits it.
NAMES = {"it": "Wikizionario", "en": "Wiktionary", "fr": "Wiktionnaire", "de": "Deutsches Wiktionary",
         "es": "Wikcionario", "pt": "Wikcionário"}
# wiktextract's parts of speech, as wdict.py lists them. The rest (prefixes,
# symbols, letters) aren't looked up as words.
POS = {"noun": "n", "name": "n", "verb": "v", "adj": "a", "adv": "r", "adv_phrase": "r",
       "phrase": "p", "prep_phrase": "e", "intj": "i", "pron": "o", "prep": "e", "conj": "c",
       "article": "t", "abbrev": "b"}
MAX_SENSES = 8
MAX_RELATED = 12
# Translations kept per language, and words kept per translation the other
# way round.
MAX_TRANSLATIONS = 6
# Glosses the extractor couldn't make out: links to further reading.
BROKEN_GLOSS = re.compile(r"\(\s*(approfondimento|citazioni)\s*\)")
# Synonyms that are a word or a phrase, not a question or a fragment.
RELATED_WORD = re.compile(r"^[\w'’ -]{2,40}$")
# A translation that's a word or a short phrase, in whatever script: it
# starts and ends with a letter, and has nothing in it that's an aside.
TRANSLATION = re.compile(r"^[^\W\d_](?:[^\n|\x1e\t()\[\]{}:;/]{0,38}[^\W\d_])?$")
# Rows of an inflection table that aren't forms of the word ("essere", the
# auxiliary a verb takes).
NOT_FORMS = {"auxiliary", "table-tags", "inflection-template", "class"}
# The languages translations are kept in: those WinDict translates between
# (src/translate.rs), by the codes the extracts give them.
TARGETS = {code: code for code in ["en", "it", "fr", "de", "es", "pt", "nl", "sv", "da", "no", "fi",
                                   "pl", "cs", "ro", "hu", "el", "tr", "ru", "uk", "ja", "ko"]}
TARGETS.update({"zh": "zh-CN", "cmn": "zh-CN", "nb": "no", "nn": "no"})
FREQUENCY = Path(__file__).resolve().parent.parent / "data" / "frequency.bin"


def open_extract(path):
    return gzip.open(path, "rt", encoding="utf-8") if path.endswith(".gz") else open(path, encoding="utf-8")


def related(words, word):
    out = []
    for w in words:
        w = " ".join((w.get("word") or "").split())
        if RELATED_WORD.match(w) and w.lower() != word.lower() and w not in out:
            out.append(w)
    return out[:MAX_RELATED]


def is_form(sense):
    return "form-of" in sense.get("tags", []) or bool(sense.get("form_of"))


def load_ranks(language):
    """The rank of each of `language`'s commonest words, from the lists
    glossing uses (see tools/build_frequency.py), or nothing without them."""
    if not FREQUENCY.is_file():
        return {}
    data = FREQUENCY.read_bytes()
    at = 0
    while at < len(data):
        n = data[at]
        code = data[at + 1:at + 1 + n].decode("ascii")
        at += 1 + n
        length = struct.unpack_from("<I", data, at)[0]
        at += 4
        if code == language:
            words = zlib.decompress(data[at:at + length], -15).decode("utf-8").split("\n")
            return {w: i for i, w in enumerate(words)}
        at += length
    return {}


def main(path, language, out):
    synsets = []  # (lemmas, definition, examples)
    antonyms = []  # per synset, per lemma
    entries = {}
    # Inflected forms: form -> bases, from form-of senses, then inflection tables.
    bases = {}
    spelt = {}  # key -> ways it's written
    page = {}  # key -> its page's (synonyms, antonyms)
    reverse = {}  # (code, foreign word in lowercase) -> keys it translates

    for line in open_extract(path):
        e = json.loads(line)
        if e.get("lang_code") != language or not e.get("word"):
            continue
        word, pos = e["word"].strip(), POS.get(e.get("pos"))
        key = word.lower()
        senses = e.get("senses", [])
        for s in senses:
            if is_form(s):
                for base in s.get("form_of", []):
                    b = (base.get("word") or "").strip().lower()
                    if b and b != key:
                        bases.setdefault(key, []).append(b)
        if pos is None:
            continue
        defined = []
        for s in senses:
            glosses = [g.strip() for g in s.get("glosses", []) if g.strip()]
            if is_form(s) or not glosses or BROKEN_GLOSS.search(glosses[0]):
                continue
            examples = [x["text"].strip() for x in s.get("examples", []) if (x.get("text") or "").strip()]
            defined.append((glosses[-1].rstrip(" ;:,"), examples))
        # The word in the other languages, in the order of its senses.
        found = []
        for t in e.get("translations", []):
            code = TARGETS.get(t.get("lang_code"))
            translation = " ".join((t.get("word") or "").split())
            if code and code != language and TRANSLATION.match(translation):
                found.append((code, translation))
        if not defined and not found:
            continue
        entry = entries.setdefault(key, new_entry())
        spelt.setdefault(key, []).append(word)
        for code, translation in found:
            words = entry["translations"].setdefault(code, [])
            if translation not in words:
                words.append(translation)
            keys = reverse.setdefault((code, translation.lower()), [])
            if key not in keys:
                keys.append(key)
        # A page whose glosses the extractor couldn't make out ("casa") is
        # kept for its translations, with nothing to define it by.
        if not defined:
            continue
        for f in e.get("forms", []):
            form = (f.get("form") or "").strip().lower()
            if NOT_FORMS & set(f.get("tags", [])):
                continue
            if form and form != key and RELATED_WORD.match(form):
                bases.setdefault(form, []).append(key)

        # A page's synonyms and opposites are the same for each of its parts
        # of speech.
        page.setdefault(key, (related(e.get("synonyms", []), word), related(e.get("antonyms", []), word)))
        ids = []
        for definition, examples in defined[:MAX_SENSES]:
            ids.append(len(synsets))
            synsets.append(([word], definition, examples))
            antonyms.append([[]])
        # The same part of speech twice (two etymologies): one list of senses.
        for i, (p, total, sense_ids) in enumerate(entry["pos"]):
            if p == pos:
                entry["pos"][i] = (p, total + len(defined), (sense_ids + ids)[:MAX_SENSES])
                break
        else:
            entry["pos"].append((pos, len(defined), ids))

    for key, entry in entries.items():
        # Keep the lowercase form unless the word only appears capitalised ("Roma").
        entry["display"] = key if key in spelt[key] else spelt[key][0]
        for code, words in entry["translations"].items():
            del words[MAX_TRANSLATIONS:]
        if not entry["pos"]:
            continue
        # Synonyms and opposites are listed in the order of the page's parts
        # of speech, so the first ones are for the first.
        synonyms, opposites = page[key]
        first = entry["pos"][0][2][0]
        lemmas, definition, examples = synsets[first]
        synsets[first] = (lemmas + synonyms, definition, examples)
        antonyms[first] = [opposites] + [[] for _ in synonyms]

    # The other way round, the commonest words first: "house" is "casa"
    # before it's "magione"; then those it's the first translation of.
    ranks = load_ranks(language)
    unranked = len(ranks)

    def best_first(code, foreign):
        def key_of(key):
            words = [w.lower() for w in entries[key]["translations"].get(code, [])]
            position = words.index(foreign) if foreign in words else len(words)
            senses = sum(total for _, total, _ in entries[key]["pos"])
            return (ranks.get(key, unranked), position, -senses, key)
        return key_of

    for (code, foreign), keys in reverse.items():
        keys.sort(key=best_first(code, foreign))
        reverse[(code, foreign)] = [entries[k]["display"] for k in keys[:MAX_TRANSLATIONS]]

    for form, found in bases.items():
        entry = entries.setdefault(form, new_entry())
        for b in found:
            kept = b in entries and (entries[b]["pos"] or entries[b]["translations"])
            if kept and b not in entry["redirects"]:
                entry["redirects"].append(b)
    for key in [k for k, e in entries.items()
                if not e["pos"] and not e["redirects"] and not e["translations"]]:
        del entries[key]

    add_spellings(entries)
    info = {"language": language, "name": NAMES.get(language, "Wiktionary"),
            "link": f"https://{language}.wiktionary.org/wiki/{{}}"}
    write(out, entries, synsets, antonyms, info, reverse)


if __name__ == "__main__":
    main(*sys.argv[1:])
