# VITRIOL — GTX 960 Dedicated Setup

## Purpose

Reserve the GTX 960 (PCI 0000:02:00.0) for VITRIOL at boot time.
The nvidia driver only claims the 1070 Ti for display. VITRIOL owns the 960.

## Setup (done 2026-05-15)

### Files installed:

| File | Purpose |
|------|---------|
| `/etc/modules-load.d/vitriol.conf` | Loads vitriol.ko at boot |
| `/etc/udev/rules.d/99-vitriol-gtx960.rules` | Forces GTX 960 to vitriol before nvidia claims it |
| `/etc/initramfs-tools/modules` | Includes vitriol in initramfs |

### What happens at boot:

1. Kernel boots, loads initramfs (contains vitriol.ko)
2. vitriol module loads early (from initramfs + modules-load.d)
3. PCI subsystem enumerates GTX 960 (device 10de:1401)
4. udev rule fires → `driver_override=vitriol` → bind to vitriol
5. nvidia loads later → only claims 1070 Ti (device 10de:1b82)
6. `/dev/vitriol` is available — BIND is no longer needed

## Verify it's working

```fish
lspci -k | grep -A2 "02:00"
# Should show: Kernel driver in use: vitriol

ls -la /dev/vitriol
# Should exist

dmesg | grep vitriol
# Should show: "GPU configured successfully"
```

## Run a DMA test

```fish
# Load module
sudo rmmod vitriol 2>/dev/null; sudo insmod vitriol-daemon/vitriol.ko
sudo chmod 666 /dev/vitriol

# Run executor (GPU is already claimed — no --bind needed)
./alka-executor/alka-executor test_p2p.alkas alka-handoff/gtx960_2gb.alkavl \
  --source llama.cpp/models/ggml-vocab-gemma-4.gguf

# Check if DMA went to VRAM
dmesg | tail -10 | grep -i vitriol
# Look for: "FLOW transferred 4096/4096 bytes"
# Look for: "BAR 1 (Data) mapped at ... [WC]"

# Verify with readback
./vitriol_readback llama.cpp/models/ggml-vocab-gemma-4.gguf
# Should print: "PASS: All 4096 bytes match GGUF source — DMA is CORRECT"
```

## Rollback (return GTX 960 to nvidia)

```fish
# 1. Remove udev rule
sudo rm /etc/udev/rules.d/99-vitriol-gtx960.rules

# 2. Remove auto-load config
sudo rm /etc/modules-load.d/vitriol.conf

# 3. Remove from initramfs
sudo sed -i '/^vitriol$/d' /etc/initramfs-tools/modules
sudo update-initramfs -u

# 4. Reboot
sudo reboot

# After reboot, check:
lspci -k | grep -A2 "02:00"
# Should show: Kernel driver in use: nvidia
```

## Without reboot: switch from vitriol to nvidia on a running system

If you need to return the 960 to nvidia right now (no reboot):

```fish
# 1. Unload vitriol (releases the GPU)
sudo rmmod vitriol

# 2. Clear driver_override so nvidia can reclaim
echo "" | sudo tee /sys/bus/pci/devices/0000:02:00.0/driver_override

# 3. Rescan PCI bus — nvidia picks it up
echo 1 | sudo tee /sys/bus/pci/rescan

# 4. Check
lspci -k | grep -A2 "02:00"
# Should show: Kernel driver in use: nvidia
```

## Switch from nvidia back to vitriol (running system)

```fish
# 1. Load vitriol
sudo insmod vitriol-daemon/vitriol.ko

# 2. Set driver_override
echo "vitriol" | sudo tee /sys/bus/pci/devices/0000:02:00.0/driver_override

# 3. If nvidia refs are low, try unbind:
echo "0000:02:00.0" | sudo tee /sys/bus/pci/drivers/nvidia/unbind 2>/dev/null

# 4. If unbind hangs, use hot-remove:
echo "1" | sudo tee /sys/bus/pci/devices/0000:02:00.0/remove
echo "1" | sudo tee /sys/bus/pci/rescan

# 5. Verify
lspci -k | grep -A2 "02:00"
# Should show: Kernel driver in use: vitriol
```
