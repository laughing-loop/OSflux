#!/usr/bin/env bash
# Rebuilds the WIM images used by the WIM split tests.
#
# Each file is stored as a "sparse" text file that keeps only its non-zero
# 512-byte sectors.
#
# Needs: wimlib-imagex (wimtools), genisoimage, python3.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

to_sparse() { # file name
    python3 - "$1" "$here/$2.sparse" <<'EOF'
import sys
source, target = sys.argv[1], sys.argv[2]
with open(source, 'rb') as image:
    data = image.read()
lines = ['osflux-sparse-image 1', f'size {len(data)}']
zero = bytes(512)
for offset in range(0, len(data), 512):
    block = data[offset:offset + 512]
    if block != zero:
        lines.append(f'{offset} {block.hex()}')
with open(target, 'w') as out:
    out.write('\n'.join(lines) + '\n')
EOF
}

# A small Windows-style image: LZX compression, several files of different
# sizes, so it can be split into several parts with a small part size.
mkdir -p "$work/tree/sub"
python3 - "$work/tree" <<'EOF'
import sys, pathlib
tree = pathlib.Path(sys.argv[1])
for index in range(6):
    lines = ''.join(f'line {line} of file {index} ' + 'x' * (index * 7) + '\n'
                    for line in range(800 + index * 300))
    (tree / f'file{index}.txt').write_text(lines)
(tree / 'sub' / 'nested.txt').write_text('nested ' * 2000)
EOF
wimlib-imagex capture "$work/tree" "$work/small.wim" "Test" --compress=LZX >/dev/null
to_sparse "$work/small.wim" small-lzx.wim

# The same files as one solid resource, as Media Creation Tool ESD files are.
wimlib-imagex capture "$work/tree" "$work/solid.wim" "Test" --solid >/dev/null
to_sparse "$work/solid.wim" small-solid.wim

# A Windows-like ISO with that image as sources/install.wim.
mkdir -p "$work/iso/efi/boot" "$work/iso/sources"
printf 'loader' >"$work/iso/efi/boot/bootx64.efi"
printf 'setup' >"$work/iso/setup.exe"
cp "$work/small.wim" "$work/iso/sources/install.wim"
genisoimage -quiet -udf -input-charset utf-8 -V CCCOMA_X64FRE_EN-US_DV9 \
    -o "$work/with-wim.iso" "$work/iso"
to_sparse "$work/with-wim.iso" windows-with-wim-udf

echo "fixtures written to $here"
