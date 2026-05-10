# VITRIOL Kernel Module Makefile
# Compile on host system with: make KDIR=/lib/modules/$(uname -r)/build

# Kernel build directory
KDIR ?= /lib/modules/$(shell uname -r)/build

# Current directory
PWD := $(shell pwd)

# Module name
MODULE_NAME := vitriol

# Build target
obj-m := $(MODULE_NAME).o

# Extra build flags
ccflags-y := -Werror -Wall -I$(PWD)

# Default target
all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

# Clean
clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

# Install
install:
	$(MAKE) -C $(KDIR) M=$(PWD) modules_install

# Load module
load:
	sudo insmod $(MODULE_NAME).ko
	dmesg | tail -20

# Unload module
unload:
	sudo rmmod $(MODULE_NAME)

# Check module info
info:
	sudo modinfo $(MODULE_NAME).ko