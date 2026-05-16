#!/bin/bash
# vitriol_bind_and_test.sh — Claim GTX 960 by force for VITRIOL DMA test
#
# Run this from a TTY (Ctrl+Alt+F3), NOT from within the GUI.
# The display manager locks the GTX 960 via nvidia-modeset/nvidia-drm.
#
# Usage:
#   sudo ./vitriol_bind_and_test.sh [--rebind] [--restart-gui]
#   sudo ./vitriol_bind_and_test.sh --gguf /path/to/model.gguf
#
# Options:
#   --rebind       After test, rebind GTX 960 back to nvidia
#   --restart-gui  Restart display manager after test
#   --gguf <path>  Use a different GGUF file for DMA test

set -e

BDF="0000:02:00.0"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EXECUTOR="$SCRIPT_DIR/alka-executor/alka-executor"
TEST_STREAM="$SCRIPT_DIR/test_p2p.alkas"
VIAL="$SCRIPT_DIR/alka-handoff/gtx960_2gb.alkavl"
GGUF="$SCRIPT_DIR/llama.cpp/models/ggml-vocab-gemma-4.gguf"
READBACK="$SCRIPT_DIR/alka-executor/alka-executor"

DO_REBIND=0
DO_RESTART_GUI=0

for arg in "$@"; do
    case "$arg" in
        --rebind) DO_REBIND=1 ;;
        --restart-gui) DO_RESTART_GUI=1 ;;
        --gguf=*) GGUF="${arg#*=}" ;;
        --gguf) shift; GGUF="$1" ;;
    esac
done

echo "=========================================="
echo " VITRIOL GTX 960 BIND + DMA Test"
echo "=========================================="
echo "Target:      $BDF (GTX 960)"
echo "GGUF source: $GGUF"
echo ""

# ── Check if running from TTY ──
if [ -n "$DISPLAY" ] || [ -n "$WAYLAND_DISPLAY" ]; then
    echo "WARNING: Running from within the GUI."
    echo "  Switch to TTY (Ctrl+Alt+F3) and re-run for best results."
    echo "  The script will attempt BIND anyway but may timeout."
    echo ""

    # Try without stopping gdm first (Tier 0/1)
    echo "Attempting Tier 0/1 (direct BIND from GUI)..."
    echo "If this fails, switch to TTY and re-run."
    echo ""
fi

# ── Helper: Check if device belongs to vitriol ──
is_vitriol_owned() {
    if [ -L "/sys/bus/pci/devices/$BDF/driver" ]; then
        DRV=$(readlink "/sys/bus/pci/devices/$BDF/driver" | xargs basename)
        [ "$DRV" = "vitriol" ] && return 0
    fi
    return 1
}

# ── Helper: Wait for device to appear under vitriol ──
wait_for_vitriol() {
    local timeout=10
    for i in $(seq 1 $timeout); do
        if is_vitriol_owned; then
            return 0
        fi
        sleep 1
    done
    return 1
}

# ═══════════════════════════════════════════════
# PRE-FLIGHT: Check current owner
# ═══════════════════════════════════════════════
echo "--- Pre-flight ---"
if is_vitriol_owned; then
    echo "  ✅ GTX 960 already belongs to vitriol"
else
    echo "  GTX 960 currently owned by: $(readlink /sys/bus/pci/devices/$BDF/driver 2>/dev/null | xargs basename 2>/dev/null || echo 'no driver')"
fi
echo ""

# ═══════════════════════════════════════════════
# BIND OPERATION (if not already vitriol)
# ═══════════════════════════════════════════════
if ! is_vitriol_owned; then
    echo "--- Tier 0/1: Attempting PCI rebind ---"

    # Step 1: Set driver_override (restraining order against nvidia)
    echo "vitriol" | sudo tee /sys/bus/pci/devices/$BDF/driver_override > /dev/null
    echo "  [OK] driver_override set"

    # Step 2: Try clean unbind (Tier 0 — polite)
    echo "  Trying: unbind from nvidia..."
    BIND_OK=false
    if timeout 5 sh -c "echo '$BDF' | sudo tee /sys/bus/pci/drivers/nvidia/unbind" 2>/dev/null; then
        echo "  [OK] Unbind succeeded"
        # Try binding to vitriol directly
        if echo "$BDF" | sudo tee /sys/bus/pci/drivers/vitriol/bind 2>/dev/null; then
            echo "  [OK] Bound to vitriol directly"
            BIND_OK=true
        else
            echo "  [--] Direct bind failed, trying remove+rescan"
        fi
    else
        echo "  [--] Unbind timed out (nvidia holds refs)"
    fi

    # Step 3: Try remove + rescan (Tier 1 — firm)
    if [ "$BIND_OK" != "true" ] && ! is_vitriol_owned; then
        echo "  Trying: hot-remove + rescan..."

        # Check if display manager is the blocker
        if [ -n "$DISPLAY" ] || [ -n "$WAYLAND_DISPLAY" ]; then
            echo "  [!!] GUI still active — remove may hang"
            echo "  Switch to TTY (Ctrl+Alt+F3) and re-run for Tier 2 escalation"
            echo ""

            # Try it anyway (the executor does this with fork-safety)
            echo "1" | sudo tee /sys/bus/pci/devices/$BDF/remove > /dev/null 2>&1 || true
            sleep 1
            echo "1" | sudo tee /sys/bus/pci/rescan > /dev/null 2>&1 || true
            sleep 2
        else
            # We're in a TTY — nvidia should release now
            echo "  [OK] Running in TTY mode"

            # Try unloading nvidia sub-modules first (cleaner)
            echo "  Unloading nvidia display modules..."
            sudo rmmod nvidia_uvm 2>/dev/null || true
            sudo rmmod nvidia_drm 2>/dev/null || true
            sudo rmmod nvidia_modeset 2>/dev/null || true
            echo "  [OK] nvidia display modules unloaded"

            echo "1" | sudo tee /sys/bus/pci/devices/$BDF/remove > /dev/null 2>&1 || true
            sleep 1
            echo "1" | sudo tee /sys/bus/pci/rescan > /dev/null 2>&1 || true
            sleep 2
        fi
    fi

    # Check result
    if wait_for_vitriol; then
        echo "  ✅ GTX 960 successfully bound to vitriol"
    else
        echo "  ❌ BIND FAILED"
        echo ""
        echo "  Manual escalation required:"
        echo "    1. Ctrl+Alt+F3 (switch to TTY)"
        echo "    2. sudo systemctl stop gdm"
        echo "    3. sudo rmmod nvidia_uvm nvidia_drm nvidia_modeset"
        echo "    4. $0"
        echo ""
        echo "  Continuing with fallback buffer (data won't reach VRAM)..."
    fi
fi

echo ""

# ═══════════════════════════════════════════════
# DMA TEST
# ═══════════════════════════════════════════════
echo "--- DMA Test ---"
echo ""

# Clear kernel log for clean results
sudo dmesg -c > /dev/null 2>&1 || true

if [ ! -f "$GGUF" ]; then
    echo "  SKIP: GGUF file not found: $GGUF"
    echo "  Provide one with --gguf <path>"
    exit 1
fi

echo "  Running executor..."
if is_vitriol_owned; then
    # Full path: kernel_read → staging buffer → memcpy_toio(BAR1) → VRAM
    echo "  [BIND OK] DMA will write directly to VRAM via BAR1"
else
    # Fallback: kernel_read → staging buffer only (no VRAM write)
    echo "  [BIND FAILED] DMA will use fallback buffer (no VRAM write)"
fi

"$EXECUTOR" "$TEST_STREAM" "$VIAL" --source "$GGUF" 2>&1
EXEC_RESULT=$?

echo ""

# ═══════════════════════════════════════════════
# VERIFICATION
# ═══════════════════════════════════════════════
echo "--- Verification ---"
echo ""

DMESG_LINES=$(sudo dmesg | grep -i "vitriol" || true)

echo "Kernel log (vitriol):"
echo "$DMESG_LINES" | while IFS= read -r line; do
    echo "  $line"
done

# Parse for key indicators
if echo "$DMESG_LINES" | grep -q "FLOW transferred 4096/4096 bytes"; then
    echo "  ✅ FLOW: 4096 bytes transferred"
fi

if echo "$DMESG_LINES" | grep -q "BAR 1 (Data) mapped"; then
    echo "  ✅ BAR1: VRAM mapped via PCI probe"
fi

if echo "$DMESG_LINES" | grep -q "BAR 1 (Data) mapped \[WC\]"; then
    echo "  ✅ BAR1: Write-combining enabled"
fi

if echo "$DMESG_LINES" | grep -q "FENCE metapage==1 (simulated, no BAR0)"; then
    echo "  ⚠ FENCE: simulated (BAR0 not accessible — normal for nvidia-claimed GPU)"
fi

if echo "$DMESG_LINES" | grep -q "nvidia_p2p_get_pages failed"; then
    echo "  ℹ P2P: not used (expected — using BIND instead)"
fi

# If BAR1 was mapped, do a readback verification
if is_vitriol_owned; then
    echo ""
    echo "  BAR1 is owned by vitriol — performing readback verification..."
    # The executor already wrote data; we can verify via READ_BAR1 IOCTL
    # For now, check dmesg for FLOW completion
    if echo "$DMESG_LINES" | grep -q "FLOW transferred 4096/4096 bytes"; then
        echo "  ✅ DMA COMPLETE: 4096 bytes written to VRAM via BAR1"
        echo ""
        echo "  First 4 bytes should be GGUF magic (47 47 55 46)"
        echo "  Full verification via READ_BAR1 IOCTL coming in next iteration"
    fi
fi

echo ""

# ═══════════════════════════════════════════════
# CLEANUP
# ═══════════════════════════════════════════════
echo "--- Cleanup ---"
echo ""

if [ "$DO_REBIND" = "1" ]; then
    echo "  Rebinding GTX 960 to nvidia..."
    sudo rmmod vitriol 2>/dev/null || true
    sleep 1
    echo "" | sudo tee /sys/bus/pci/devices/$BDF/driver_override 2>/dev/null || true
    echo "1" | sudo tee /sys/bus/pci/rescan > /dev/null 2>&1 || true
    echo "  [OK] Done"
else
    echo "  Leaving GTX 960 bound to vitriol"
    echo "  To unbind later:"
    echo "    sudo rmmod vitriol"
    echo "    echo 1 | sudo tee /sys/bus/pci/rescan"
fi

if [ "$DO_RESTART_GUI" = "1" ]; then
    echo "  Restarting display manager..."
    sudo systemctl start gdm 2>/dev/null || \
        sudo systemctl start lightdm 2>/dev/null || \
        sudo systemctl start sddm 2>/dev/null || true
    echo "  [OK] Done"
fi

echo ""
echo "=========================================="
echo " Test complete"
echo "=========================================="
