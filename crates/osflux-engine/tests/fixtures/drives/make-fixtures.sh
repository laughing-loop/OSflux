#!/usr/bin/env bash
# Rebuilds the drive images used by the drive contents tests.
#
# Each image is a 1 GiB drive built with the standard Linux tools, then stored
# as a "sparse" text file that keeps only its non-zero 512-byte sectors, so a
# few kilobytes stand in for the whole drive.
#
# Needs: sfdisk (fdisk), sgdisk (gdisk), mkfs.fat (dosfstools),
# mkfs.exfat (exfatprogs), mtools, python3.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

drive_size=$((1024 * 1024 * 1024))
start_sector=2048
start_bytes=$((start_sector * 512))

new_drive() {
    rm -f "$1"
    truncate -s "$drive_size" "$1"
}

mbr_one_partition() { # drive type
    printf 'label: dos\nstart=%s, type=%s\n' "$start_sector" "$2" | sfdisk -q "$1"
}

fat32_in_partition() { # drive label [partition sectors]
    local sectors=${3:-$(((drive_size - start_bytes) / 512))}
    # mkfs.fat counts the filesystem size in 1 KiB blocks.
    mkfs.fat -F 32 -n "$2" --offset "$start_sector" "$1" "$((sectors / 2))" >/dev/null
}

exfat_in_partition() { # drive label
    local part="$work/part.img"
    rm -f "$part"
    truncate -s $((drive_size - start_bytes)) "$part"
    mkfs.exfat -L "$2" "$part" >/dev/null
    dd if="$part" of="$1" bs=1M seek=1 conv=sparse,notrunc status=none
}

to_sparse() { # drive name
    python3 - "$1" "$here/$2.sparse" <<'EOF'
import sys
source, target = sys.argv[1], sys.argv[2]
with open(source, 'rb') as drive:
    data = drive.read()
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

drive="$work/drive.img"
mtools_drive() { echo "$drive@@$start_bytes"; }
export MTOOLS_SKIP_CHECK=1

# An MBR with no partitions, as diskpart's clean leaves a removable drive.
new_drive "$drive"
printf 'label: dos\n' | sfdisk -q "$drive"
to_sparse "$drive" mbr-empty

# One empty FAT32 partition, as Windows formats a new stick.
new_drive "$drive"
mbr_one_partition "$drive" c
fat32_in_partition "$drive" KINGSTON
to_sparse "$drive" mbr-fat32-empty

# The same, plus the System Volume Information folder Windows adds.
mmd -i "$(mtools_drive)" "::/System Volume Information"
printf '{6A1E9F2C-0000-0000-0000-000000000000}' >"$work/IndexerVolumeGuid"
mcopy -i "$(mtools_drive)" "$work/IndexerVolumeGuid" "::/System Volume Information/"
to_sparse "$drive" mbr-fat32-svi

# A file that was copied on and then deleted again.
printf 'old notes' >"$work/notes.txt"
mcopy -i "$(mtools_drive)" "$work/notes.txt" "::/"
mdel -i "$(mtools_drive)" "::/notes.txt"
to_sparse "$drive" mbr-fat32-deleted

# A file in the root folder.
printf 'not really a photo' >"$work/Holiday Photo.jpg"
mcopy -i "$(mtools_drive)" "$work/Holiday Photo.jpg" "::/"
to_sparse "$drive" mbr-fat32-file

# A recycle bin folder, which can hold deleted files.
new_drive "$drive"
mbr_one_partition "$drive" c
fat32_in_partition "$drive" KINGSTON
mmd -i "$(mtools_drive)" "::/\$RECYCLE.BIN"
to_sparse "$drive" mbr-fat32-recycle-bin

# One empty FAT32 partition on a GPT drive.
new_drive "$drive"
sgdisk -n "1:$start_sector:0" -t 1:0700 "$drive" >/dev/null
# The partition ends before the 33 sectors of the backup GPT at the end.
fat32_in_partition "$drive" GPTSTICK $((drive_size / 512 - 34 - start_sector))
to_sparse "$drive" gpt-fat32-empty

# One empty exFAT partition, as many 64 GB+ sticks ship.
new_drive "$drive"
mbr_one_partition "$drive" 7
exfat_in_partition "$drive" SANDISK
to_sparse "$drive" mbr-exfat-empty

# Two empty FAT32 partitions.
new_drive "$drive"
printf 'label: dos\nstart=2048, size=1048576, type=c\nstart=1050624, type=c\n' | sfdisk -q "$drive"
mkfs.fat -F 32 -n FIRST --offset 2048 "$drive" 524288 >/dev/null
mkfs.fat -F 32 -n SECOND --offset 1050624 "$drive" 523264 >/dev/null
to_sparse "$drive" mbr-two-fat32

# FAT32 written straight onto the drive, with no partition table.
new_drive "$drive"
mkfs.fat -F 32 -n NOTABLE "$drive" >/dev/null
to_sparse "$drive" superfloppy-fat32

# One empty FAT16 partition.
new_drive "$drive"
printf 'label: dos\nstart=2048, size=262144, type=6\n' | sfdisk -q "$drive"
mkfs.fat -F 16 -n SMALL --offset 2048 "$drive" 131072 >/dev/null
to_sparse "$drive" mbr-fat16-empty

echo "fixtures written to $here"
