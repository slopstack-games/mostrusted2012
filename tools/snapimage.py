"""Carve the decrypted game image out of a memory snapshot into a PE that Ghidra (and REA) can load.

    python3 tools/snapimage.py re/out/live/snap1 re/out/live/NFS13_snap1.exe [--functions re/NFS13_dump/functions.csv]

A snapshot (`mwtool memscan` / the probe: `mem.bin` plus `index.tsv`, one line per mapping: start, end, file
offset, protection, path) holds the running game's image at 0x400000 with `.text` decrypted and `.data` / `.bss`
as they were at that moment. The image is written in its memory layout, so the section table is rewritten with
file offsets equal to virtual addresses.

`--functions` (the dump's function table) adds an export for every code address that a data section points at
(vtables, callback and state tables) but that is not a known function start: Ghidra's analysis of the dump did
not find them (docs/reverse-engineering.md "Functions the dump is missing"), and Ghidra creates a function at
every export. Only addresses after padding (int3 / nop / ret) count, so jump-table targets inside functions
stay out. The output is derived from the game: keep it in the git-ignored re/out.
"""
import argparse, csv, struct

BASE = 0x400000
EXE = 'NFS13.exe'


def carve(snap):
    """The image's bytes: the snapshot's NFS13.exe mappings from 0x400000, contiguous in mem.bin."""
    maps = []
    for line in open(f'{snap}/index.tsv'):
        f = line.rstrip('\n').split('\t')
        if len(f) >= 3:
            maps.append((int(f[0], 16), int(f[1], 16), int(f[2]), f[4] if len(f) > 4 else ''))
    first = next(m for m in maps if m[0] == BASE and m[3].endswith(EXE))
    end = max(e for s, e, _, p in maps if p.endswith(EXE) and s >= BASE)
    for s, e, off, _ in maps:  # every mapping in the range must sit at the same distance in the file
        if BASE <= s < end:
            assert off - first[2] == s - BASE, f'mapping {s:#x} is not contiguous in mem.bin'
    with open(f'{snap}/mem.bin', 'rb') as f:
        f.seek(first[2])
        img = bytearray(f.read(end - BASE))
    assert img[:2] == b'MZ'
    return img


def headers(img):
    pe = struct.unpack_from('<I', img, 0x3c)[0]
    assert img[pe:pe + 4] == b'PE\0\0'
    nsec, optsz = struct.unpack_from('<H', img, pe + 6)[0], struct.unpack_from('<H', img, pe + 20)[0]
    return pe, pe + 24, pe + 24 + optsz, nsec


def memory_layout(img):
    """Raw offsets = virtual addresses, raw sizes = section-aligned virtual sizes."""
    _, opt, sec, nsec = headers(img)
    align = struct.unpack_from('<I', img, opt + 32)[0]
    struct.pack_into('<I', img, opt + 36, align)
    for i in range(nsec):
        o = sec + 40 * i
        vsize, va = struct.unpack_from('<II', img, o + 8)
        struct.pack_into('<II', img, o + 16, min((vsize + align - 1) // align * align, len(img) - va), va)


def sections(img):
    _, _, sec, nsec = headers(img)
    out = []
    for i in range(nsec):
        o = sec + 40 * i
        name = img[o:o + 8].rstrip(b'\0').decode('latin1')
        vsize, va = struct.unpack_from('<II', img, o + 8)
        flags = struct.unpack_from('<I', img, o + 36)[0]
        out.append((name, va, vsize, flags))
    return out


def missing_starts(img, functions):
    known = set()
    with open(functions, newline='') as f:
        for row in csv.DictReader(f):
            known.add(int(row['address'], 16))
    secs = sections(img)
    code = [(BASE + va, BASE + va + vs) for _, va, vs, fl in secs if fl & 0x20]  # IMAGE_SCN_CNT_CODE
    data = [(va, va + vs) for _, va, vs, fl in secs if not fl & 0x20 and fl & 0x40]  # initialised data
    found = set()
    for lo, hi in data:
        for off in range(lo, min(hi, len(img)) - 3, 4):
            v = struct.unpack_from('<I', img, off)[0]
            if v in known or not any(a <= v < b for a, b in code):
                continue
            o = v - BASE
            if img[o - 1] in (0xcc, 0x90, 0xc3) or img[o - 3] == 0xc2:
                found.add(v)
    return sorted(found)


def add_exports(img, addrs, dll):
    """Append an `.edata` section exporting `vt_<address>` for each address."""
    pe, opt, sec, nsec = headers(img)
    vs, va = struct.unpack_from('<II', img, sec + 40 * (nsec - 1) + 8)
    new_va = (va + vs + 0xfff) // 0x1000 * 0x1000
    assert new_va == len(img)
    n = len(addrs)
    names = [f'vt_{a:08x}'.encode() + b'\0' for a in addrs]  # fixed width: sorted as the name table must be
    dll = dll.encode() + b'\0'
    eat, npt = 40, 40 + 4 * n
    ords = npt + 4 * n
    dll_at = ords + 2 * n
    name_at = dll_at + len(dll)
    blob = bytearray(name_at + sum(map(len, names)))
    struct.pack_into('<IIHHIIIIIII', blob, 0, 0, 0, 0, 0, new_va + dll_at, 1, n, n, new_va + eat, new_va + npt,
                     new_va + ords)
    blob[dll_at:dll_at + len(dll)] = dll
    p = name_at
    for i, (a, nm) in enumerate(zip(addrs, names)):
        struct.pack_into('<I', blob, eat + 4 * i, a - BASE)
        struct.pack_into('<I', blob, npt + 4 * i, new_va + p)
        struct.pack_into('<H', blob, ords + 2 * i, i)
        blob[p:p + len(nm)] = nm
        p += len(nm)
    size = (len(blob) + 0xfff) // 0x1000 * 0x1000
    blob += bytes(size - len(blob))
    o = sec + 40 * nsec
    assert o + 40 <= struct.unpack_from('<I', img, opt + 60)[0] and img[o:o + 40] == bytes(40)
    img[o:o + 40] = struct.pack('<8sIIIIIIHHI', b'.edata', len(blob), new_va, size, new_va, 0, 0, 0, 0, 0x40000040)
    struct.pack_into('<H', img, pe + 6, nsec + 1)
    struct.pack_into('<I', img, opt + 56, new_va + size)
    struct.pack_into('<II', img, opt + 96, new_va, len(blob))
    img += blob


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('snapshot')
    ap.add_argument('out')
    ap.add_argument('--functions', help="the dump's functions.csv: export the function starts it is missing")
    a = ap.parse_args()
    img = carve(a.snapshot)
    memory_layout(img)
    for name, va, vs, _ in sections(img):
        print(f'{name or "(none)":8} {BASE + va:#010x} {vs:#x}')
    if a.functions:
        addrs = missing_starts(img, a.functions)
        add_exports(img, addrs, a.out.rsplit('/', 1)[-1])
        print(f'{len(addrs)} pointer-referenced function starts missing from {a.functions}, exported')
    open(a.out, 'wb').write(img)


if __name__ == '__main__':
    main()
