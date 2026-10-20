#!/usr/bin/env python3
"""Converts a Wiktionary extract by wiktextract (kaikki.org) into WinDict's
compact dictionary, for the words of one language.

Usage: python tools/build_wiktionary.py <extract.jsonl[.gz]> <language> <out.wdict>

For a dictionary in the language it defines, use that language's own
Wiktionary, e.g. the Italian one's for Italian:
https://kaikki.org/itwiktionary/raw-wiktextract-data.jsonl.gz

The file format is described in `wdict.py`, which writes it.
"""

import gzip
import json
import re
import sys

from wdict import add_spellings, new_entry, write

# Each Wiktionary's name, as the card credits it.
NAMES = {"it": "Wikizionario", "en": "Wiktionary", "fr": "Wiktionnaire", "de": "Wiktionary",
         "es": "Wikcionario", "pt": "Wikcionário"}
# wiktextract's parts of speech, as wdict.py lists them. The rest (prefixes,
# symbols, letters) aren't looked up as words.
POS = {"noun": "n", "name": "n", "verb": "v", "adj": "a", "adv": "r", "adv_phrase": "r",
       "phrase": "p", "prep_phrase": "e", "intj": "i", "pron": "o", "prep": "e", "conj": "c",
       "article": "t", "abbrev": "b"}
MAX_SENSES = 8
MAX_RELATED = 12
# Glosses the extractor couldn't make out: links to further reading.
BROKEN_GLOSS = re.compile(r"\(\s*(approfondimento|citazioni)\s*\)")
# Synonyms that are a word or a phrase, not a question or a fragment.
RELATED_WORD = re.compile(r"^[\w'’ -]{2,40}$")
# Rows of an inflection table that aren't forms of the word ("essere", the
# auxiliary a verb takes).
NOT_FORMS = {"auxiliary", "table-tags", "inflection-template", "class"}


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


def main(path, language, out):
    synsets = []  # (lemmas, definition, examples)
    antonyms = []  # per synset, per lemma
    entries = {}
    # Inflected forms: form -> bases, from form-of senses, then inflection tables.
    bases = {}
    spelt = {}  # key -> ways it's written
    page = {}  # key -> its page's (synonyms, antonyms)

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
        if not defined:
            continue
        for f in e.get("forms", []):
            form = (f.get("form") or "").strip().lower()
            if NOT_FORMS & set(f.get("tags", [])):
                continue
            if form and form != key and RELATED_WORD.match(form):
                bases.setdefault(form, []).append(key)

        entry = entries.setdefault(key, new_entry())
        spelt.setdefault(key, []).append(word)
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
        # Synonyms and opposites are listed in the order of the page's parts
        # of speech, so the first ones are for the first.
        synonyms, opposites = page[key]
        first = entry["pos"][0][2][0]
        lemmas, definition, examples = synsets[first]
        synsets[first] = (lemmas + synonyms, definition, examples)
        antonyms[first] = [opposites] + [[] for _ in synonyms]

    for form, found in bases.items():
        entry = entries.setdefault(form, new_entry())
        for b in found:
            if b in entries and entries[b]["pos"] and b not in entry["redirects"]:
                entry["redirects"].append(b)
    for key in [k for k, e in entries.items() if not e["pos"] and not e["redirects"]]:
        del entries[key]

    add_spellings(entries)
    info = {"language": language, "name": NAMES.get(language, "Wiktionary"),
            "link": f"https://{language}.wiktionary.org/wiki/{{}}"}
    write(out, entries, synsets, antonyms, info)


if __name__ == "__main__":
    main(*sys.argv[1:])
