"""Checks that the executables take their DLLs from System32 only.

Usage: python tools/check_imports.py <exe>...

Each must be linked with /DEPENDENTLOADFLAG:0x800, so that Windows looks
for the DLLs it imports in System32 and not beside the executable (the
setup and the portable copy are run from the Downloads folder), and must
carry the C runtime rather than import it, so that it starts on a PC
without the Visual C++ redistributable. Exits with 1 if any doesn't.
"""

import struct
import sys

SYSTEM32_ONLY = 0x800
LOAD_CONFIG = 10
# Where DependentLoadFlags is in a 64-bit IMAGE_LOAD_CONFIG_DIRECTORY.
DEPENDENT_LOAD_FLAGS = 0x4E


def read(path):
    """The DLLs `path` imports, and its DependentLoadFlags (None if it has none)."""
    data = open(path, "rb").read()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe : pe + 4] != b"PE\0\0":
        raise ValueError("not an executable")
    sections = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError("not a 64-bit executable")
    directories = optional + 112
    table = []
    for i in range(sections):
        at = optional + optional_size + 40 * i
        size, address, raw_size, raw = struct.unpack_from("<IIII", data, at + 8)
        table.append((address, max(size, raw_size), raw))

    def offset(rva):
        for address, size, raw in table:
            if address <= rva < address + size:
                return rva - address + raw
        raise ValueError("an address outside the file's sections")

    imports = []
    rva = struct.unpack_from("<I", data, directories + 8)[0]
    at = offset(rva) if rva else None
    while at is not None:
        name = struct.unpack_from("<I", data, at + 12)[0]
        if not name:
            break
        start = offset(name)
        imports.append(data[start : data.index(b"\0", start)].decode("ascii"))
        at += 20
    flags = None
    rva = struct.unpack_from("<I", data, directories + 8 * LOAD_CONFIG)[0]
    if rva:
        at = offset(rva)
        if struct.unpack_from("<I", data, at)[0] >= DEPENDENT_LOAD_FLAGS + 2:
            flags = struct.unpack_from("<H", data, at + DEPENDENT_LOAD_FLAGS)[0]
    return imports, flags


def problems(path):
    imports, flags = read(path)
    found = []
    if flags != SYSTEM32_ONLY:
        found.append(f"DependentLoadFlags is {flags}, not {SYSTEM32_ONLY:#x}")
    runtime = [
        dll
        for dll in imports
        if dll.lower().startswith(("vcruntime", "msvcp", "api-ms-win-crt-"))
    ]
    if runtime:
        found.append("imports the C runtime: " + ", ".join(runtime))
    return found


def main(paths):
    if not paths:
        print(__doc__)
        return 2
    failed = False
    for path in paths:
        try:
            found = problems(path)
        except (OSError, ValueError, struct.error) as e:
            found = [str(e)]
        for problem in found:
            print(f"{path}: {problem}")
        if not found:
            print(f"{path}: ok")
        failed = failed or bool(found)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
