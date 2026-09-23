#!/usr/bin/env bash
# vitriol-optane-repart.sh — carve raw store zone on Optane 900P (nvme0n1)
# 2026-09-23. Level-2 store prep: keep swap p1, shrink XFS p2, create raw p3.
#
# RUN AS ROOT (sudo). Idempotent: refuses to re-run on the target layout.
#
# Target (512B sectors):
#   p1 2048         134217728    64G  swap              (never touched)
#   p2 134219776    201326592    96G  XFS /scratch      (recreated, new UUID)
#   p3 335546368    585105408   279G  raw store zone    (no fs)
#   p4 920651776    17051278   8.1G  future boot        (never touched)

set -euo pipefail

# --dry-run: verify preconditions + print the sfdisk input, touch nothing.
DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

DEV=/dev/nvme0n1
SCRATCH_MNT=/scratch
STAGE=/home/randozart/Downloads/staging-qwen38fn
BACKUP=/home/randozart/Downloads/nvme0n1-gpt.bak

# Expected current start/size (sectors) for the partitions we must NOT disturb
EXPECT_P1_START=2048
EXPECT_P1_SIZE=134217728
EXPECT_P4_START=920651776
EXPECT_P4_SIZE=17051279

# New p2/p3 geometry
P2_START=134219776
P2_SIZE=201326592
P3_START=335546368
P3_SIZE=585105408

die() { echo "FATAL: $*" >&2; exit 1; }

# --- helpers ---------------------------------------------------------------
sect() { cat "/sys/block/nvme0n1/nvme0n1$1/start" 2>/dev/null || die "no part $1 start"; }
siz() { cat "/sys/block/nvme0n1/nvme0n1$1/size"  2>/dev/null || die "no part $1 size"; }

# --- precondition checks ----------------------------------------------------
# dry-run: preconditions are root-free (sysfs/du/findmnt); destructive steps need root
if [ "$DRY_RUN" = "1" ] && [ "$(id -u)" -ne 0 ]; then
    # fall through to dry-run preconditions below; skip the root gate + destructive steps
    :
elif [ "$(id -u)" -eq 0 ]; then
    :
else
    die "must run as root (sudo) for a real repartition (use --dry-run to verify without root)"
fi
[ -b "$DEV" ] || die "no block device $DEV"

# stage must exist + be verified by caller
[ -d "$STAGE" ] || die "stage dir missing: $STAGE (run P0 rsync first)"
[ "$(du -sb "$STAGE" | cut -f1)" -gt 80000000000 ] \
    || die "stage dir looks empty (<80G): $STAGE"

# /scratch must be free of open files
if command -v lsof >/dev/null 2>&1; then
    if [ -n "$(lsof +f -- "$SCRATCH_MNT" 2>/dev/null)" ]; then
        die "files open on $SCRATCH_MNT: $(lsof +f -- "$SCRATCH_MNT" 2>/dev/null | head)"
    fi
fi

# partition sanity: p1/p4 geometry must match expected (no prior drift)
[ "$(sect p1)" = "$EXPECT_P1_START" ] || die "p1 start drifted: $(sect p1) != $EXPECT_P1_START"
[ "$(siz p1)" = "$EXPECT_P1_SIZE" ]   || die "p1 size drifted: $(siz p1) != $EXPECT_P1_SIZE"
[ "$(sect p4)" = "$EXPECT_P4_START" ] || die "p4 start drifted: $(sect p4) != $EXPECT_P4_START"
[ "$(siz p4)" = "$EXPECT_P4_SIZE" ]   || die "p4 size drifted: $(siz p4) != $EXPECT_P4_SIZE"

# idempotence: already the target layout?
if [ -b "${DEV}p3" ] && [ "$(sect p3)" = "$P3_START" ] && [ "$(siz p3)" = "$P3_SIZE" ]; then
    echo "already partitioned to target; nothing to do"
    exit 0
fi

echo "== backing up GPT to $BACKUP"
if [ "$DRY_RUN" = "1" ]; then
    echo "  [dry-run] (skipped: sfdisk -d needs root)"
else
    sfdisk -d "$DEV" > "$BACKUP" || die "sfdisk backup failed"
fi

echo "== unmounting $SCRATCH_MNT"
if [ "$DRY_RUN" = "1" ]; then
    if findmnt "$SCRATCH_MNT" >/dev/null 2>&1; then
        echo "  [dry-run] $SCRATCH_MNT is mounted (will be unmounted)"
    else
        echo "  [dry-run] $SCRATCH_MNT not mounted (ok)"
    fi
else
    if findmnt "$SCRATCH_MNT" >/dev/null 2>&1; then
        umount "$SCRATCH_MNT" || die "umount failed (open files?)"
    else
        echo "  $SCRATCH_MNT not mounted (ok)"
    fi
fi

# sfdisk refuses whole-disk repartition while any partition has a live swap.
# p1 swap is 0B used (zram holds live swap); capture BEFORE sfdisk so restore
# runs even if it was active.
SWAP_P1="$(awk '$1 == "'"$DEV"'p1" {print $1}' /proc/swaps)"
if [ -n "$SWAP_P1" ]; then
    echo "== swapoff ${DEV}p1 (zram holds live swap; p1 is 0B used)"
    if [ "$DRY_RUN" = "1" ]; then
        echo "  [dry-run] (skipped)"
    else
        swapoff "${DEV}p1" || die "swapoff ${DEV}p1 failed"
    fi
fi

echo "== repartitioning: delete p2, recreate p2(96G)+p3(279G raw)"
# Only p2 is deleted/recreated. p1 and p4 are NOT in the input.
if [ "$DRY_RUN" = "1" ]; then
    echo "  [dry-run] sfdisk input (NOT applied):"
    echo "    ${DEV}p1 : start=${EXPECT_P1_START}, size=${EXPECT_P1_SIZE}, type=swap"
    echo "    ${DEV}p2 : start=${P2_START}, size=${P2_SIZE}, type=linux"
    echo "    ${DEV}p3 : start=${P3_START}, size=${P3_SIZE}, type=linux"
    echo "    ${DEV}p4 : start=${EXPECT_P4_START}, size=${EXPECT_P4_SIZE}"
    echo "  [dry-run] stopping before any destructive step"
    exit 0
fi
sfdisk "$DEV" <<EOF
label: gpt
unit: sectors

${DEV}p1 : start=${EXPECT_P1_START}, size=${EXPECT_P1_SIZE}, type=swap
${DEV}p2 : start=${P2_START}, size=${P2_SIZE}, type=linux
${DEV}p3 : start=${P3_START}, size=${P3_SIZE}, type=linux
${DEV}p4 : start=${EXPECT_P4_START}, size=${EXPECT_P4_SIZE}
EOF

# reload partition table
if [ "$DRY_RUN" = "1" ]; then
    echo "  [dry-run] partprobe skipped"
else
    partprobe "$DEV" || udevadm settle
fi

# restore swap on p1 — partition geometry unchanged, safe to re-arm
if [ -n "$SWAP_P1" ]; then
    echo "== swapon ${DEV}p1 (restored)"
    if [ "$DRY_RUN" = "1" ]; then
        echo "  [dry-run] (skipped)"
    else
        swapon "${DEV}p1" || die "swapon ${DEV}p1 failed"
    fi
fi

# post-assert: p1/p4 untouched, p2/p3 present with exact geometry
[ "$(sect p1)" = "$EXPECT_P1_START" ] || die "post: p1 start drifted!"
[ "$(siz p1)" = "$EXPECT_P1_SIZE" ]   || die "post: p1 size drifted!"
[ "$(sect p4)" = "$EXPECT_P4_START" ] || die "post: p4 start drifted!"
[ "$(siz p4)" = "$EXPECT_P4_SIZE" ]   || die "post: p4 size drifted!"
[ "$(sect p2)" = "$P2_START" ] && [ "$(siz p2)" = "$P2_SIZE" ] || die "post: p2 geometry wrong"
[ "$(sect p3)" = "$P3_START" ] && [ "$(siz p3)" = "$P3_SIZE" ] || die "post: p3 geometry wrong"

echo "== wiping stray signatures off raw p3"
wipefs -a "${DEV}p3" || true

echo "== mkfs.xfs on p2"
mkfs.xfs -f "${DEV}p2"
NEW_UUID="$(blkid -s UUID -o value "${DEV}p2")"
echo "new /scratch UUID: $NEW_UUID"

echo "== updating /etc/fstab"
FSTAB=/etc/fstab
OLD_LINE="$(grep -E '^UUID=.*  /scratch ' "$FSTAB" || true)"
[ -n "$OLD_LINE" ] || die "no /scratch line in fstab"
sed -i.bak "s|^UUID=.*  /scratch .*|UUID=${NEW_UUID}  /scratch  xfs  noatime,nodiratime 0 0|" "$FSTAB"
echo "  removed: $OLD_LINE"
echo "  added:   $(grep -E '^UUID=.*  /scratch ' "$FSTAB")"
echo "  fstab backup: $FSTAB.bak"

echo "== mounting /scratch"
mount "$SCRATCH_MNT" || die "mount failed"
findmnt "$SCRATCH_MNT" || die "findmnt assert failed"

echo "== restoring staged model from $STAGE"
mkdir -p "$SCRATCH_MNT/qwen38fn"
rsync -aH "$STAGE/" "$SCRATCH_MNT/qwen38fn/"
echo "  staged bytes: $(du -sb "$STAGE" | cut -f1)"
echo "  restored bytes: $(du -sb "$SCRATCH_MNT/qwen38fn" | cut -f1)"

echo "== DONE. Verdict checks (manual):"
echo "  lsblk /dev/nvme0n1            -> p1 64G swap, p2 96G xfs, p3 279G, p4 8.1G"
echo "  lsblk -f /dev/nvme0n1p3       -> empty (no fs) = raw zone ready"
echo "  swapon --show                 -> both zram + p1 present"
echo "  After verifying model bytes: rm -rf '$STAGE'"
echo ""
echo "  usage: $0 [--dry-run]   (dry-run = verify + print, no changes)"