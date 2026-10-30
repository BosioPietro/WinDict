#!/usr/bin/env python3
"""Writes the lists of each language's common words that glossing a passage
underlines the others of, from wordfreq's data.

Usage: python tools/build_frequency.py <wordfreq data dir> data/frequency.bin

The data is in the `wordfreq` package on PyPI (wordfreq/data in its wheel),
built from Wikipedia, subtitles, news, books, the web and social media:
https://github.com/rspeer/wordfreq. Reading it takes `msgpack`.

File layout (little endian), a section per language, one after the other:

    u8 code_length   code (ASCII, as the translation languages name it)
    u32 data_length  raw deflate of the words, '\\n'-separated, commonest first

Words are as wordfreq has them: casefolded (Turkish keeps its dotted and
dotless i apart), split at hyphens, and in Japanese, Chinese and Korean cut
into the pieces a word is made of. Those with digits are left out.
"""

import gzip
import struct
import sys
import zlib
from pathlib import Path

import msgpack

# The translation languages, as code and wordfreq's name for it. Each takes
# as many of its commonest words as it needs for those of a passage a
# learner knows: more the more forms a word takes.
LANGUAGES = [
    ("en", "en", 8_000),
    ("it", "it", 12_000),
    ("fr", "fr", 12_000),
    ("de", "de", 14_000),
    ("es", "es", 12_000),
    ("pt", "pt", 12_000),
    ("nl", "nl", 12_000),
    ("sv", "sv", 12_000),
    ("da", "da", 12_000),
    ("no", "nb", 12_000),
    ("fi", "fi", 18_000),
    ("pl", "pl", 18_000),
    ("cs", "cs", 18_000),
    ("ro", "ro", 12_000),
    ("hu", "hu", 18_000),
    ("el", "el", 18_000),
    ("tr", "tr", 18_000),
    ("ru", "ru", 18_000),
    ("uk", "uk", 18_000),
    ("ja", "ja", 7_000),
    ("ko", "ko", 12_000),
    ("zh-CN", "zh", 7_000),
]


def words(data_dir, name):
    """Every word wordfreq has in the language, commonest first."""
    for size in ("large", "small"):
        path = data_dir / f"{size}_{name}.msgpack.gz"
        if path.exists():
            # A header, then a list of words for each frequency, highest first.
            bins = msgpack.unpackb(gzip.decompress(path.read_bytes()), raw=False)[1:]
            return [w for b in bins for w in b]
    raise FileNotFoundError(f"no list for {name} in {data_dir}")


def common(data_dir, name, count):
    kept = []
    for word in words(data_dir, name):
        if any(c.isdigit() for c in word) or not any(c.isalpha() for c in word):
            continue
        kept.append(word)
        if len(kept) == count:
            break
    return kept


def main():
    data_dir, out = Path(sys.argv[1]), Path(sys.argv[2])
    sections = []
    for code, name, count in LANGUAGES:
        kept = common(data_dir, name, count)
        packer = zlib.compressobj(9, zlib.DEFLATED, -15)
        packed = packer.compress("\n".join(kept).encode()) + packer.flush()
        sections.append(bytes([len(code)]) + code.encode() + struct.pack("<I", len(packed)) + packed)
        print(f"{code}: {len(kept)} words, {len(packed):,} bytes")
    out.write_bytes(b"".join(sections))
    print(f"{out}: {out.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
