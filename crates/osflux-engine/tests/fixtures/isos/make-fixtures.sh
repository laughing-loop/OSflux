#!/usr/bin/env bash
# Rebuilds the ISO images used by the UDF reader tests.
#
# Each image is built with genisoimage and stored as a "sparse" text file
# that keeps only its non-zero 512-byte sectors.
#
# Needs: genisoimage, python3.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

to_sparse() { # image name
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

# A tree laid out like a Windows installer ISO, with contents the tests can
# predict byte for byte.
tree="$work/tree"
mkdir -p "$tree/boot" "$tree/efi/boot" "$tree/sources" "$tree/support/logging"
printf '[AutoRun.Amd64]\r\nopen=setup.exe\r\nicon=setup.exe,0\r\n' >"$tree/autorun.inf"
printf 'bootmgr test\n' >"$tree/bootmgr"
python3 - "$tree" <<'EOF'
import sys, pathlib
tree = pathlib.Path(sys.argv[1])
# Spans five 2048-byte blocks.
(tree / 'setup.exe').write_bytes(bytes((index * 7 + 3) % 256 for index in range(9000)))
(tree / 'efi/boot/bootx64.efi').write_bytes(bytes(index % 251 for index in range(3000)))
# A large file of zeros, read back from sparse fixture data.
(tree / 'sources/boot.wim').write_bytes(bytes(3 * 1024 * 1024))
(tree / 'sources/empty.txt').write_bytes(b'')
# Enough long names that the folder listing spans several blocks.
for number in range(1, 41):
    name = f'Microsoft-Windows-Setup-Component-Log-Number-{number:02}.etl'
    (tree / 'support/logging' / name).write_text(f'log {number:02}\n')
(tree / 'support' / 'résumé.txt').write_text('café', encoding='utf-8')
(tree / 'support' / '日本語.txt').write_text('nihongo', encoding='utf-8')
EOF

genisoimage -quiet -udf -input-charset utf-8 -V CCCOMA_X64FRE_EN-US_DV9 \
    -o "$work/windows-like.iso" "$tree"
to_sparse "$work/windows-like.iso" windows-like-udf

# Shaped like Microsoft's Windows ISOs: the ISO 9660 directory holds only a
# README, every other file is in UDF only, and El Torito has BIOS and UEFI
# entries.
udfonly="$work/udf-only"
mkdir -p "$udfonly/boot" "$udfonly/efi/boot" "$udfonly/efi/microsoft/boot"
printf 'This disc contains a "UDF" file system.\r\n' >"$udfonly/README.TXT"
head -c 2048 /dev/zero | tr '\0' 'B' >"$udfonly/boot/etfsboot.com"
head -c 2048 /dev/zero | tr '\0' 'E' >"$udfonly/efi/microsoft/boot/efisys.bin"
printf 'loader' >"$udfonly/efi/boot/bootx64.efi"
genisoimage -quiet -udf -input-charset utf-8 -V CCCOMA_X64FRE_EN-US_DV9 \
    -b boot/etfsboot.com -no-emul-boot -boot-load-size 4 -c boot/boot.cat -hide boot.cat \
    -eltorito-alt-boot -e efi/microsoft/boot/efisys.bin -no-emul-boot \
    -hide boot -hide efi -o "$work/udf-only.iso" "$udfonly"
to_sparse "$work/udf-only.iso" windows-udf-only

# A plain ISO 9660 image, with no UDF.
mkdir -p "$work/plain"
printf 'readme\n' >"$work/plain/readme.txt"
genisoimage -quiet -input-charset utf-8 -V PLAIN_ISO -o "$work/plain.iso" "$work/plain"
to_sparse "$work/plain.iso" plain-iso9660

echo "fixtures written to $here"
